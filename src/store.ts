/**
 * The store: an app definition plus a History, with a movable head.
 *
 * The head is the time the UI shows. In live mode it follows the newest event,
 * like a primary database. backtrack(t) moves it into the past without
 * discarding anything, like Aurora Backtrack. Dispatching while backtracked
 * forks history: events after the head are dropped, as a rewind would be.
 *
 * A whole session can be exported as bytes and imported elsewhere. Because the
 * bytes contain every event with its time, the import reproduces the exact same
 * states on any machine. Attach the bytes to a bug report; replay them in a test.
 */
import {
  BytesReader,
  BytesWriter,
  CodecError,
  type Codec,
  encode,
  fnv1a64,
  fromBase64,
  toBase64,
} from "./codec.js";
import { History, type EventRecord } from "./log.js";

export interface AppDefinition<S, E> {
  /** Stable name. Written into exported sessions so a wrong import is caught early. */
  readonly name: string;
  /** Wire format of the event body. Usually a `variant({...})`. */
  readonly events: Codec<E>;
  /** Initial state. Called once per store. */
  readonly init: () => S;
  /** Pure function. Must return a new state, never mutate the old one. */
  readonly apply: (state: S, event: EventRecord<E>) => S;
  /** Save a checkpoint every N events. Default 64. */
  readonly checkpointInterval?: number;
}

export interface StoreOptions {
  /** Where this store's events come from. Default: random 8 hex characters. */
  origin?: string;
  /** Source of time for dispatch(). Default Date.now. Inject a fake clock in tests. */
  clock?: () => number;
}

export type Listener<S> = (state: S, reason: ChangeReason) => void;
export type ChangeReason = "dispatch" | "backtrack" | "advance" | "live" | "import" | "prune";

const MAGIC = 0x52475241; // "RGRA"
const FORMAT_VERSION = 1;

export class Store<S, E> {
  readonly origin: string;
  private readonly clock: () => number;
  private history: History<S, E>;
  private seq = 0;
  private headTime = Infinity;
  private listeners = new Set<Listener<S>>();
  private cached: { n: number; state: S } | undefined;

  constructor(
    readonly definition: AppDefinition<S, E>,
    options: StoreOptions = {},
  ) {
    this.origin = options.origin ?? randomOrigin();
    this.clock = options.clock ?? (() => Date.now());
    this.history = new History(definition.init(), definition.apply, definition.checkpointInterval ?? 64);
  }

  // ----- reading -----------------------------------------------------------

  /** True when the head follows the newest event. */
  get live(): boolean {
    return this.headTime === Infinity;
  }

  /** The time the UI currently shows. In live mode, the newest event time. */
  get head(): number {
    return this.live ? this.history.latestTime : this.headTime;
  }

  /** State at the head. Cached until the history or head changes. */
  get state(): S {
    const n = this.history.countThrough(this.head);
    if (this.cached && this.cached.n === n) return this.cached.state;
    const state = this.history.stateAfter(n);
    this.cached = { n, state };
    return state;
  }

  /** State at any retained time. Does not move the head. */
  stateAt(time: number): S {
    return this.history.stateAt(time);
  }

  /** Every retained event, oldest first. */
  events(): readonly EventRecord<E>[] {
    return this.history.all();
  }

  /** Events with from < time <= to. Handy for "what changed between these two frames". */
  eventsBetween(from: number, to: number): readonly EventRecord<E>[] {
    return this.history.between(from, to);
  }

  /** Events whose apply() threw. They are retained but skipped. */
  get faults(): readonly { id: string; error: unknown }[] {
    return this.history.faults;
  }

  get earliestRetainedTime(): number {
    return this.history.earliestRetainedTime;
  }

  get latestTime(): number {
    return this.history.latestTime;
  }

  // ----- writing -----------------------------------------------------------

  /**
   * Record an event. Returns the record so callers can keep its id.
   * When the head is in the past, every event after the head is dropped first;
   * the new event starts a new future, like a rewind in a text editor.
   */
  dispatch(body: E, time: number = this.clock()): EventRecord<E> {
    if (!this.live) this.truncateAfterHead();
    const record = this.record(body, time);
    this.history.insert(record);
    this.changed("dispatch");
    return record;
  }

  /**
   * Insert an event produced elsewhere (another tab, a server, a replay).
   * Returns false if the id was already known. Late events are placed at their
   * own time and later state is rebuilt.
   */
  receive(record: EventRecord<E>): boolean {
    const expected = computeId(this.definition.events, record);
    if (expected !== record.id) {
      throw new CodecError(`event id ${record.id} does not match its content (${expected})`);
    }
    const pos = this.history.insert(record);
    if (pos === -1) return false;
    this.changed("dispatch");
    return true;
  }

  // ----- time travel -------------------------------------------------------

  /** Show the state as of `time`. Nothing is deleted. */
  backtrack(time: number): void {
    this.headTime = time;
    this.changed("backtrack");
  }

  /** Alias for backtrack, named as contime names it. */
  advanceTo(time: number): void {
    this.headTime = time;
    this.changed("advance");
  }

  /** Return to following the newest event. */
  goLive(): void {
    this.headTime = Infinity;
    this.changed("live");
  }

  /** Forget events at or before `before`, folding them into the base state. */
  prune(before: number): number {
    const n = this.history.prune(before);
    if (n > 0) this.changed("prune");
    return n;
  }

  /**
   * An independent store with the same history, like an Aurora clone.
   * Useful for "what if" explorations in devtools or tests.
   */
  fork(options: StoreOptions = {}): Store<S, E> {
    const s = new Store(this.definition, { origin: options.origin ?? randomOrigin(), clock: options.clock ?? this.clock });
    s.history = this.history.clone();
    s.headTime = this.headTime;
    return s;
  }

  // ----- subscriptions -----------------------------------------------------

  subscribe(listener: Listener<S>): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  // ----- sessions ----------------------------------------------------------

  /**
   * Serialize the retained history and the head.
   * Layout: magic u32, version u8, app name, head (i64, or i64 min for live),
   * then u32 count and each event as (time i64, origin string, seq u32, body).
   * Event ids are not stored; they are recomputed on import and must match.
   */
  export(): Uint8Array {
    const w = new BytesWriter(1024);
    w.u32(MAGIC);
    w.u8(FORMAT_VERSION);
    w.string(this.definition.name);
    w.bool(this.live);
    w.i64(this.live ? 0 : this.headTime);
    const events = this.history.all();
    w.u32(events.length);
    for (const ev of events) {
      w.i64(ev.time);
      w.string(ev.origin);
      w.u32(ev.seq);
      this.definition.events.write(w, ev.body);
    }
    return w.finish();
  }

  /** Base64 of export(), for pasting into an issue or a log line. */
  exportText(): string {
    return toBase64(this.export());
  }

  /** Rebuild a store from export() bytes. The result is bit-for-bit the same history. */
  static import<S, E>(definition: AppDefinition<S, E>, bytes: Uint8Array, options: StoreOptions = {}): Store<S, E> {
    const r = new BytesReader(bytes);
    if (r.u32() !== MAGIC) throw new CodecError("not a reagera session (bad magic)");
    const version = r.u8();
    if (version !== FORMAT_VERSION) throw new CodecError(`unsupported session version ${version}`);
    const name = r.string();
    if (name !== definition.name) throw new CodecError(`session is for app "${name}", not "${definition.name}"`);
    const live = r.bool();
    const head = r.i64();
    const count = r.u32();
    const store = new Store(definition, options);
    for (let i = 0; i < count; i++) {
      const time = r.i64();
      const origin = r.string();
      const seq = r.u32();
      const body = definition.events.read(r);
      const partial = { time, origin, seq, body };
      const id = computeId(definition.events, partial);
      store.history.insert({ id, ...partial });
      if (origin === store.origin && seq >= store.seq) store.seq = seq + 1;
    }
    if (r.remaining !== 0) throw new CodecError(`${r.remaining} trailing bytes after session`);
    store.headTime = live ? Infinity : head;
    store.changed("import");
    return store;
  }

  static importText<S, E>(definition: AppDefinition<S, E>, text: string, options: StoreOptions = {}): Store<S, E> {
    return Store.import(definition, fromBase64(text.trim()), options);
  }

  // ----- internals ---------------------------------------------------------

  private record(body: E, time: number): EventRecord<E> {
    const partial = { time, origin: this.origin, seq: this.seq++, body };
    return { id: computeId(this.definition.events, partial), ...partial };
  }

  private truncateAfterHead(): void {
    const keep = this.history.all().slice(0, this.history.countThrough(this.headTime));
    const fresh = new History<S, E>(this.definition.init(), this.definition.apply, this.definition.checkpointInterval ?? 64);
    for (const ev of keep) fresh.insert(ev);
    this.history = fresh;
    this.headTime = Infinity;
  }

  private changed(reason: ChangeReason): void {
    this.cached = undefined;
    const state = this.state;
    for (const l of this.listeners) l(state, reason);
  }
}

export function computeId<E>(codec: Codec<E>, ev: { time: number; origin: string; seq: number; body: E }): string {
  const w = new BytesWriter(64);
  w.i64(ev.time);
  w.string(ev.origin);
  w.u32(ev.seq);
  codec.write(w, ev.body);
  return fnv1a64(w.finish());
}

/** Bytes of a single event body, for logging or network transport. */
export function encodeBody<E>(definition: AppDefinition<unknown, E>, body: E): Uint8Array {
  return encode(definition.events, body);
}

export function defineApp<S, E>(definition: AppDefinition<S, E>): AppDefinition<S, E> {
  return definition;
}

export function createStore<S, E>(definition: AppDefinition<S, E>, options?: StoreOptions): Store<S, E> {
  return new Store(definition, options);
}

function randomOrigin(): string {
  const bytes = new Uint8Array(4);
  if (typeof crypto !== "undefined" && crypto.getRandomValues) crypto.getRandomValues(bytes);
  else for (let i = 0; i < 4; i++) bytes[i] = Math.floor(Math.random() * 256);
  let s = "";
  for (const b of bytes) s += b.toString(16).padStart(2, "0");
  return s;
}
