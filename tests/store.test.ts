import { test } from "node:test";
import assert from "node:assert/strict";
import { Store, computeId, createStore, type EventRecord } from "../src/index.js";
import { Counter, Events, ticking, type Event } from "./counter.js";

test("state is a fold over the log", () => {
  const store = createStore(Counter, { origin: "a", clock: ticking() });
  store.dispatch({ type: "add", payload: 2 });
  store.dispatch({ type: "add", payload: 3 });
  assert.equal(store.state.count, 5);
  assert.equal(store.events().length, 2);
  assert.equal(store.head, 1010);
});

test("stateAt answers any retained time without moving the head", () => {
  const store = createStore(Counter, { origin: "a", clock: ticking(1000, 10) });
  for (let i = 1; i <= 10; i++) store.dispatch({ type: "add", payload: i }); // times 1000..1090
  assert.equal(store.stateAt(999).count, 0);
  assert.equal(store.stateAt(1000).count, 1);
  assert.equal(store.stateAt(1045).count, 1 + 2 + 3 + 4 + 5);
  assert.equal(store.stateAt(5000).count, 55);
  assert.equal(store.state.count, 55);
  assert.ok(store.live);
});

test("checkpoints give the same answers as a full replay", () => {
  const withCp = createStore({ ...Counter, checkpointInterval: 3 }, { origin: "a", clock: ticking() });
  const noCp = createStore({ ...Counter, checkpointInterval: 1_000_000 }, { origin: "a", clock: ticking() });
  for (let i = 1; i <= 50; i++) {
    const body: Event = i % 7 === 0 ? { type: "reset", payload: undefined } : { type: "add", payload: i };
    withCp.dispatch(body);
    noCp.dispatch(body);
  }
  for (let t = 990; t <= 1500; t += 7) assert.equal(withCp.stateAt(t).count, noCp.stateAt(t).count, `t=${t}`);
});

test("backtrack shows the past, goLive returns, nothing is lost", () => {
  const store = createStore(Counter, { origin: "a", clock: ticking() });
  const seen: string[] = [];
  store.subscribe((_, reason) => seen.push(reason));
  store.dispatch({ type: "add", payload: 1 });
  store.dispatch({ type: "add", payload: 1 });
  store.dispatch({ type: "add", payload: 1 });
  store.backtrack(1010);
  assert.equal(store.state.count, 2);
  assert.equal(store.live, false);
  assert.equal(store.events().length, 3);
  store.goLive();
  assert.equal(store.state.count, 3);
  assert.deepEqual(seen, ["dispatch", "dispatch", "dispatch", "backtrack", "live"]);
});

test("dispatching while backtracked forks history", () => {
  const store = createStore(Counter, { origin: "a", clock: ticking() });
  store.dispatch({ type: "add", payload: 1 }); // 1000
  store.dispatch({ type: "add", payload: 10 }); // 1010
  store.dispatch({ type: "add", payload: 100 }); // 1020
  store.backtrack(1000);
  store.dispatch({ type: "add", payload: 5 }); // 1030
  assert.ok(store.live);
  assert.equal(store.state.count, 6);
  assert.deepEqual(store.events().map((e) => e.body.payload), [1, 5]);
});

test("late events are placed by time and later state is rebuilt", () => {
  const a = createStore(Counter, { origin: "a", clock: ticking(1000, 100) });
  const b = createStore(Counter, { origin: "b", clock: ticking(1050, 100) });
  const a1 = a.dispatch({ type: "add", payload: 1 }); // 1000
  const a2 = a.dispatch({ type: "reset", payload: undefined }); // 1100
  const b1 = b.dispatch({ type: "add", payload: 7 }); // 1050, arrives late at a
  a.receive(b1);
  assert.deepEqual(a.events().map((e) => e.id), [a1.id, b1.id, a2.id]);
  assert.equal(a.stateAt(1050).count, 8);
  assert.equal(a.state.count, 0);
  // Arrival order does not matter.
  const c = createStore(Counter, { origin: "c" });
  c.receive(a2);
  c.receive(b1);
  c.receive(a1);
  assert.deepEqual(c.events().map((e) => e.id), [a1.id, b1.id, a2.id]);
  assert.deepEqual(c.state, a.state);
});

test("duplicates are ignored and tampered ids are rejected", () => {
  const a = createStore(Counter, { origin: "a", clock: ticking() });
  const ev = a.dispatch({ type: "add", payload: 1 });
  assert.equal(a.receive(ev), false);
  assert.equal(a.state.count, 1);
  const forged: EventRecord<Event> = { ...ev, body: { type: "add", payload: 999 } };
  assert.throws(() => a.receive(forged), /does not match/);
  assert.equal(computeId(Events, ev), ev.id);
});

test("same millisecond keeps dispatch order", () => {
  const store = createStore(Counter, { origin: "a", clock: () => 1000 });
  store.dispatch({ type: "add", payload: 1 });
  store.dispatch({ type: "reset", payload: undefined });
  store.dispatch({ type: "add", payload: 2 });
  assert.equal(store.state.count, 2);
});

test("a throwing apply is recorded as a fault and skipped", () => {
  const store = createStore(Counter, { origin: "a", clock: ticking() });
  store.dispatch({ type: "add", payload: 1 });
  store.dispatch({ type: "explode", payload: undefined });
  store.dispatch({ type: "add", payload: 1 });
  assert.equal(store.state.count, 2);
  assert.equal(store.faults.length, 1);
  assert.equal(store.faults[0].id, store.events()[1].id);
});

test("export and import reproduce the identical history and states", () => {
  const store = createStore(Counter, { origin: "tab1", clock: ticking(1_700_000_000_000, 37) });
  for (let i = 1; i <= 20; i++) store.dispatch({ type: "add", payload: i });
  store.dispatch({ type: "rename", payload: { label: "héllo", note: "n" } });
  store.backtrack(1_700_000_000_000 + 37 * 5);
  const bytes = store.export();
  const copy = Store.import(Counter, bytes, { origin: "tab2" });
  assert.deepEqual(copy.events(), store.events());
  assert.equal(copy.live, false);
  assert.equal(copy.head, store.head);
  assert.deepEqual(copy.state, store.state);
  assert.deepEqual(copy.export(), bytes);
  copy.goLive();
  assert.equal(copy.state.count, 210);
  assert.equal(copy.state.label, "héllo");
  const text = store.exportText();
  assert.deepEqual(Store.importText(Counter, text).events(), store.events());
});

test("import rejects another app's session", () => {
  const store = createStore(Counter, { origin: "a" });
  store.dispatch({ type: "add", payload: 1 });
  const other = { ...Counter, name: "other" };
  assert.throws(() => Store.import(other, store.export()), /not "other"/);
});

test("importing with the same origin continues the sequence", () => {
  const store = createStore(Counter, { origin: "same", clock: ticking() });
  store.dispatch({ type: "add", payload: 1 });
  const copy = Store.import(Counter, store.export(), { origin: "same", clock: ticking(2000) });
  const ev = copy.dispatch({ type: "add", payload: 1 });
  assert.equal(ev.seq, 1);
  assert.equal(copy.state.count, 2);
});

test("fork gives an independent clone", () => {
  const store = createStore(Counter, { origin: "a", clock: ticking() });
  store.dispatch({ type: "add", payload: 1 });
  const twin = store.fork({ origin: "b" });
  twin.dispatch({ type: "add", payload: 41 });
  assert.equal(store.state.count, 1);
  assert.equal(twin.state.count, 42);
});

test("prune folds old events into the base state", () => {
  const store = createStore(Counter, { origin: "a", clock: ticking(1000, 10) });
  for (let i = 0; i < 10; i++) store.dispatch({ type: "add", payload: 1 });
  assert.equal(store.prune(1040), 5);
  assert.equal(store.events().length, 5);
  assert.equal(store.state.count, 10);
  assert.equal(store.stateAt(1040).count, 5);
  assert.equal(store.stateAt(1070).count, 8);
  assert.equal(store.earliestRetainedTime, 1040);
  assert.throws(() => store.dispatch({ type: "add", payload: 1 }, 1000), RangeError);
});

test("eventsBetween explains what changed between two frames", () => {
  const store = createStore(Counter, { origin: "a", clock: ticking(1000, 10) });
  for (let i = 0; i < 5; i++) store.dispatch({ type: "add", payload: i });
  assert.deepEqual(store.eventsBetween(1010, 1030).map((e) => e.body.payload), [2, 3]);
});
