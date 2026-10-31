# Reagera

A front-end framework where the event log is the state.

Reagera (Swedish for "react") stores every user action as a timestamped event in an
append-only log. The state you render is a fold over that log. Nothing else is the
source of truth. This is the idea behind Amazon Aurora, where the storage layer is a
redo log and every page is derived from it. Reagera applies the same idea to a UI.

Because the log is the state, three things come for free:

- Query any moment. `store.stateAt(t)` returns the state at time `t`. A slider in the
  devtools panel scrubs through the session.
- Reproduce any session. `store.export()` returns compact bytes. `Store.import()`
  rebuilds the identical history on another machine, in a test, or in a bug report.
- Accept events late or out of order. Events are ordered by their own time, never by
  arrival. A late event lands at its proper place and later state is rebuilt.

Reagera has zero runtime dependencies. The whole framework is five short files.

## Where the ideas come from

| Source | What Reagera takes from it |
| --- | --- |
| [minbin](https://github.com/humacss/minbin) | An explicit, auditable binary format with no derive magic. Big-endian integers, length-prefixed strings, tagged variants. You write the schema; the bytes follow it. |
| [contime](https://github.com/humacss/contime) | Continuous-time state: ordered histories, deterministic application, checkpoints, late events, retention through pruning. |
| Aurora | The log is the database. A movable head (Backtrack), cheap clones (fork), and a storage format that can be shipped as bytes. |

## Install

```sh
npm install reagera
```

Reagera is published as ES modules with TypeScript declarations. Node 18 or newer
and any evergreen browser work.

## A complete app

```ts
import { codec, createStore, defineApp, h, mount, type EventRecord, type Infer } from "reagera";
import { devtools } from "reagera/devtools";

// 1. The wire format of every event. This is the contract of the app.
const Events = codec.variant({
  add: codec.i64,
  reset: codec.unit,
});
type Event = Infer<typeof Events>;

// 2. The state and how each event changes it. Pure and immutable.
interface State { readonly count: number }

const Counter = defineApp<State, Event>({
  name: "counter",
  events: Events,
  init: () => ({ count: 0 }),
  apply(state, ev: EventRecord<Event>) {
    switch (ev.body.type) {
      case "add": return { count: state.count + ev.body.payload };
      case "reset": return { count: 0 };
    }
  },
});

// 3. The view: a pure function of state.
const view = (state: State, dispatch: (e: Event) => void) =>
  h("div", null,
    h("button", { onclick: () => dispatch({ type: "add", payload: -1 }) }, "-"),
    h("span", null, ` ${state.count} `),
    h("button", { onclick: () => dispatch({ type: "add", payload: 1 }) }, "+"),
    h("button", { onclick: () => dispatch({ type: "reset", payload: undefined }) }, "reset"),
  );

// 4. Mount it. The devtools panel is optional.
const store = createStore(Counter);
mount(store, view, document.getElementById("root")!);
devtools(store);
```

There are no components with local state. State that is not in the log cannot be
replayed, so Reagera does not offer a place for it. Keep transient input text in the
DOM element itself and dispatch an event when the user commits it.

## Events

An event record has five fields:

| Field | Meaning |
| --- | --- |
| `time` | Logical time in milliseconds. Events are ordered by this first. |
| `origin` | Who produced it. Each store gets a random 8-character origin unless you pass one. |
| `seq` | A per-origin counter, so two events in the same millisecond keep their order. |
| `body` | Your event, usually `{ type, payload }` from a `codec.variant`. |
| `id` | A 64-bit FNV-1a hash of the four fields above. Identical content gives an identical id. |

The canonical order is `(time, origin, seq)`. The id is used for deduplication and
for integrity: `store.receive(record)` recomputes it and rejects a record whose id
does not match its content.

`store.dispatch(body)` records an event at the clock's current time. Pass a second
argument to set the time yourself. Pass `clock` in the store options to use a fake
clock in tests.

## Time travel

```ts
store.state;          // state at the head
store.head;           // the time the UI shows
store.live;           // true when the head follows the newest event
store.backtrack(t);   // show the state as of t; nothing is deleted
store.goLive();       // follow the newest event again
store.stateAt(t);     // query any time without moving the head
store.eventsBetween(t1, t2);  // what happened between two frames
```

Dispatching while the head is in the past drops every event after the head and
starts a new future. That matches what a rewind means in an editor. Use
`store.fork()` first if you want to keep both branches. A fork is an independent
store with a copy of the history, like an Aurora clone.

## Sessions

```ts
const bytes = store.export();            // Uint8Array
const text = store.exportText();         // base64 of the same bytes
const copy = Store.import(Counter, bytes);
const same = Store.importText(Counter, text);
```

The session format is small and explicit:

```text
u32   magic "RGRA"
u8    format version (1)
string app name, checked on import
bool  live
i64   head time (0 when live)
u32   event count
      per event: i64 time, string origin, u32 seq, body in the app's codec
```

Ids are not stored. They are recomputed on import, so a session cannot carry a
record whose id disagrees with its content. An import for a different app name
fails before any event is read.

To reproduce a bug: ask the reporter to press "export" in the devtools panel and
paste the text. In a test, call `Store.importText(App, text)` and assert on
`store.state`, or on `store.stateAt(t)` for the moment the reporter describes.
The todo example goes one step further and keeps the whole session in the URL
hash, so the URL itself is the bug report.

## Faults

If `apply` throws for an event, the event is kept in the log, skipped during
replay, and listed in `store.faults` with the error. The rest of the history still
works and the devtools panel marks the event in red. The faulty event survives an
export, so the bug can be reproduced from the session.

## Checkpoints and retention

Replaying a long history for every query would be slow. The history saves a
checkpoint every `checkpointInterval` events (default 64). A query at time `t`
starts from the nearest checkpoint at or before `t`. A late event invalidates only
the checkpoints after its position.

Checkpoints hold references to states, so `apply` must return a new state and never
mutate the old one.

`store.prune(before)` folds every event at or before `before` into the base state
and forgets them. Queries can no longer look at the pruned range. Use it in a long
running app to cap memory.

## The codec

The codec follows minbin: no header, no schema metadata, no padding. You read the
schema and you know the bytes.

| Codec | Encoding |
| --- | --- |
| `u8`, `u16`, `u32`, `i64` | Fixed width, big endian. `i64` carries a JavaScript safe integer. |
| `f64` | IEEE 754, big endian |
| `bool` | One byte, 0 or 1. Other values are invalid. |
| `string` | u32 UTF-8 byte length, then the bytes |
| `list(T)` | u32 count, then the elements |
| `option(T)` | u8 tag, 0 for none, 1 followed by the value |
| `struct({...})` | Fields in the declared order |
| `variant({...})` | u8 tag equal to the position of the key, then that variant's value |
| `unit` | No bytes |

Add new variants at the end. Reordering keys changes the tags and breaks old
sessions. Put a version in your app name (`"todo/2"`) when you change the format.

`encode(codec, value)` and `decode(codec, bytes)` work on any codec, not only on
events, so you can use the same format for network messages.

## Multiple sources

Events from another tab, a worker, or a server go in through `store.receive(record)`.
The record is placed by its own time. If it is older than the head, every later
state is rebuilt. The same records received in any order produce the same state,
provided `apply` is deterministic. This is the contime guarantee.

Because origins and sequence numbers are part of the order, two clients that
dispatch at the same millisecond still agree on who went first.

## Run the examples

```sh
git clone https://github.com/isamisushi/reagera
cd reagera
npm install
npm run example
```

Open http://localhost:8080/ and pick one.

Todo is the classic app. Add some todos, then drag the slider in the panel at the
bottom right. Press "export", copy the text, and paste it into the import box of a
fresh tab to replay the session. Or copy the URL: the session lives in the hash.

Latency lab shows the part that makes Reagera different. Two clients, alice and
bob, share an arena. Each click is an event. Events reach the other client after a
delay with jitter that you control, so they often arrive out of order. A late event
is drawn with an orange ring and inserted behind newer ones. The number on each dot
is the order the history settles on. A status line turns green when alice, bob and
a reference store agree, which happens as soon as nothing is in flight, whatever
the arrival order was. The scrubber queries `stateAt(t)` on both clients without
moving their heads, and "put session in URL" makes the run reproducible.

## Tests

```sh
npm test
```

The suite runs on Node's built-in test runner with no extra dependencies. It covers
the codec byte layout, ordering of late and same-millisecond events, checkpoint
equivalence with full replay, backtrack and fork, export and import round trips,
faults, pruning, and the DOM diff.

## Limits

- The log lives in memory. Persist the export bytes yourself if you need them to
  survive a reload. The todo example uses the URL hash.
- Rendering diffs children by position within a parent. There is no keyed
  reordering yet, so a long list that reorders often will re-create its nodes.
- `i64` is limited to JavaScript safe integers (53 bits).
- The store processes events on the calling thread. There are no workers.

## License

MIT
