/**
 * The history: an ordered, deduplicated event log with checkpoints.
 *
 * This is the part borrowed from contime. Events are ordered by
 * (time, origin, seq), never by arrival order, so a late event lands in its
 * proper place and the state after it is rebuilt. Checkpoints are saved states
 * at known positions; a query at time T starts from the nearest checkpoint at
 * or before T and applies only the remaining events.
 *
 * And it is the idea behind Aurora: the log is the database. State is never
 * stored as the source of truth; it is always a fold over the log.
 */

export interface EventRecord<E> {
  /** Content ID: 16 hex characters, derived from origin, seq, time and body. */
  readonly id: string;
  /** Logical time in milliseconds. Events are ordered by this first. */
  readonly time: number;
  /** Who produced the event. Each browser tab or test gets its own origin. */
  readonly origin: string;
  /** Per-origin counter, so two events in the same millisecond keep their order. */
  readonly seq: number;
  /** The user-defined event body, usually { type, payload }. */
  readonly body: E;
}

export type Apply<S, E> = (state: S, event: EventRecord<E>) => S;

interface Checkpoint<S> {
  /** Number of events applied to reach this state. */
  readonly pos: number;
  readonly state: S;
}

export function compareEvents(a: { time: number; origin: string; seq: number }, b: { time: number; origin: string; seq: number }): number {
  if (a.time !== b.time) return a.time < b.time ? -1 : 1;
  if (a.origin !== b.origin) return a.origin < b.origin ? -1 : 1;
  if (a.seq !== b.seq) return a.seq < b.seq ? -1 : 1;
  return 0;
}

export class History<S, E> {
  private events: EventRecord<E>[] = [];
  private ids = new Set<string>();
  private checkpoints: Checkpoint<S>[] = [];
  /** State before the first retained event. Changes only when history is pruned. */
  private base: S;
  /** Time of the last pruned event, or -Infinity when nothing was pruned. */
  private baseTime = -Infinity;
  /** Events whose apply() threw, by id. Those events are skipped, not lost. */
  private faultsById = new Map<string, { id: string; error: unknown }>();

  constructor(
    init: S,
    private readonly apply: Apply<S, E>,
    private readonly checkpointInterval = 64,
  ) {
    this.base = init;
    if (checkpointInterval < 1) throw new RangeError("checkpointInterval must be at least 1");
  }

  get length(): number {
    return this.events.length;
  }

  get faults(): readonly { id: string; error: unknown }[] {
    return [...this.faultsById.values()];
  }

  /** The time of the newest retained event, or baseTime when empty. */
  get latestTime(): number {
    const n = this.events.length;
    return n === 0 ? this.baseTime : this.events[n - 1].time;
  }

  get earliestRetainedTime(): number {
    return this.baseTime;
  }

  /** A read-only view of the retained events in canonical order. */
  all(): readonly EventRecord<E>[] {
    return this.events;
  }

  /** Events with from < time <= to, in canonical order. */
  between(from: number, to: number): readonly EventRecord<E>[] {
    return this.events.slice(this.countThrough(from), this.countThrough(to));
  }

  has(id: string): boolean {
    return this.ids.has(id);
  }

  /**
   * Insert an event at its canonical position.
   * Returns the position, or -1 when the id was already present.
   * Checkpoints after the insertion point are discarded; they will be rebuilt lazily.
   */
  insert(event: EventRecord<E>): number {
    if (this.ids.has(event.id)) return -1;
    if (event.time <= this.baseTime) {
      throw new RangeError(`event ${event.id} at ${event.time} is older than retained history (${this.baseTime})`);
    }
    const pos = this.lowerBound(event);
    this.events.splice(pos, 0, event);
    this.ids.add(event.id);
    // A checkpoint at pos N is the state after the first N events.
    // Any checkpoint covering the new event's position is now stale.
    let keep = this.checkpoints.length;
    while (keep > 0 && this.checkpoints[keep - 1].pos > pos) keep--;
    this.checkpoints.length = keep;
    return pos;
  }

  /** Number of retained events with time <= t. */
  countThrough(t: number): number {
    let lo = 0;
    let hi = this.events.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (this.events[mid].time <= t) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  /** State at time t: base state folded with every retained event with time <= t. */
  stateAt(t: number): S {
    return this.stateAfter(this.countThrough(t));
  }

  /** State after the first n events. */
  stateAfter(n: number): S {
    let pos = 0;
    let state = this.base;
    const cp = this.checkpointAtOrBefore(n);
    if (cp) {
      pos = cp.pos;
      state = cp.state;
    }
    while (pos < n) {
      state = this.step(state, this.events[pos]);
      pos++;
      if (pos % this.checkpointInterval === 0 && this.lastCheckpointPos() < pos) {
        this.checkpoints.push({ pos, state });
      }
    }
    return state;
  }

  /**
   * Fold every event with time <= before into the base state and forget them.
   * Queries can no longer look at the pruned range. Mirrors contime's retention.
   */
  prune(before: number): number {
    const n = this.countThrough(before);
    if (n === 0) return 0;
    this.base = this.stateAfter(n);
    this.baseTime = this.events[n - 1].time;
    const removed = this.events.splice(0, n);
    for (const ev of removed) {
      this.ids.delete(ev.id);
      this.faultsById.delete(ev.id);
    }
    this.checkpoints = this.checkpoints
      .filter((c) => c.pos >= n)
      .map((c) => ({ pos: c.pos - n, state: c.state }));
    return n;
  }

  /** Independent copy sharing nothing mutable; states are assumed immutable. */
  clone(): History<S, E> {
    const h = new History<S, E>(this.base, this.apply, this.checkpointInterval);
    h.events = this.events.slice();
    h.ids = new Set(this.ids);
    h.checkpoints = this.checkpoints.slice();
    h.baseTime = this.baseTime;
    h.faultsById = new Map(this.faultsById);
    return h;
  }

  private step(state: S, event: EventRecord<E>): S {
    try {
      return this.apply(state, event);
    } catch (error) {
      // A faulty event must not take the whole history down. Record it and move on.
      this.faultsById.set(event.id, { id: event.id, error });
      return state;
    }
  }

  private lowerBound(event: EventRecord<E>): number {
    let lo = 0;
    let hi = this.events.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (compareEvents(this.events[mid], event) < 0) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  private checkpointAtOrBefore(n: number): Checkpoint<S> | undefined {
    for (let i = this.checkpoints.length - 1; i >= 0; i--) {
      if (this.checkpoints[i].pos <= n) return this.checkpoints[i];
    }
    return undefined;
  }

  private lastCheckpointPos(): number {
    const k = this.checkpoints.length;
    return k === 0 ? 0 : this.checkpoints[k - 1].pos;
  }
}
