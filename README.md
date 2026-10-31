# Reagera

A front-end framework where the event log is the state. Rust, compiled to WebAssembly.

Reagera records every user action as a timestamped event in a
[ConTime](https://github.com/humacss/contime) history and encodes it with
[minbin](https://github.com/humacss/minbin). The state you render is a fold over that
history. Nothing else is the source of truth. This is the idea behind Amazon Aurora,
where storage is a redo log and every page is derived from it. Reagera applies it to a UI.

Because the log is the state, four things come for free:

- Query any moment. `store.state_at(ms)` returns the state at that millisecond. The
  devtools panel has a slider that scrubs through the session.
- Reproduce any session. `store.export()` returns a few hundred bytes. `Store::import`
  rebuilds the identical history on another machine, in a test, or from a bug report.
- Accept events late or out of order. ConTime orders events by their own time, never
  by arrival. A late event lands at its proper place and later state is rebuilt.
- Replay side effects without repeating them. `apply` emits a request, the runtime
  performs it once, and the answer is a record. A replay meets the recorded answer and
  never calls the network, the clock, or the payment gateway again.

Live examples: https://reagera.fly.dev/

## How Reagera differs from React

React renders the current state. Reagera renders a point in a history. React keeps state in memory and replaces it on each update, so the previous state is gone unless you add a tool that records it. In Reagera, the history is the only source of truth, and the current state is one query against it.

| Topic | React | Reagera |
| --- | --- | --- |
| Source of truth | Current state in components or a store | An ordered log of timestamped events |
| Local state | `useState`, in any component | None. All state is in the log. |
| Past states | Not kept. Redux DevTools can record them if you use Redux. | Always available through `state_at(ms)` and `backtrack(ms)` |
| Order of updates | The order in which updates arrive | The time of each event, whatever the arrival order |
| Late or duplicate events | Application code must handle them | The history inserts late events by time and ignores duplicates |
| Side effects | `useEffect`, run from the render and ordered by a dependency array | `apply` emits a request. The answer is a record. `perform` is the one place that touches the outside world. |
| A stale response | Application code checks a ref or an abort signal | `resolve` sees the request and the state together, in log order |
| Reproducing a bug | Steps to reproduce, logs, or a session recording tool | The exported session rebuilds the same event log and the same states |
| Testing | A DOM or a simulated DOM, usually jsdom | Views compile natively. No DOM is necessary. |
| Language | JavaScript or TypeScript | Rust, compiled to WebAssembly |

A bug report is the clearest difference. In React, you try to recreate the state from a description. In Reagera, the reporter sends the session text, and you import it in a test. The session holds the answers the outside world gave, so the test sees the same declined payment and the same late response, and performs nothing.

The second difference is unreliable delivery. Reagera orders events by time, so several clients that receive the same events in a different order reach the same state. The result depends on deterministic `apply` functions and on the clocks of the clients. Reagera does not correct clock differences between clients.

Reagera is not a replacement for React in most projects. Use React if you need any of these:

- A large ecosystem of components, libraries, and developers
- Server-side rendering or a framework such as Next.js
- Keyed list reordering and concurrent rendering
- A small download. Each Reagera example is about 100 KB of WebAssembly after gzip, in addition to its JavaScript bindings.

Use Reagera if exact replay of a session is more important than these items, for example in collaborative tools, offline-first apps, or apps that have bugs that are difficult to reproduce. Reagera is an early project with three examples and no production use.

## What each dependency does

| Crate | Role in Reagera |
| --- | --- |
| contime | Is the history. There is no second copy: the event list for export and for the devtools panel is a ConTime range query. Canonical ordering by `(time, id)`, deduplication, checkpoints every N events, point-in-time queries, late insertion with rebuild. Reagera uses the `manual` driver, so there are no threads: the worker is stepped on the browser main thread until it is idle. |
| minbin | Encodes records and sessions. Fixed-width big-endian integers, length-prefixed strings, explicit enum tags. You write the schema with `minbin_enum!` or by hand; the bytes follow it. |
| wasm-bindgen, web-sys | The DOM renderer, the runtime and the devtools panel. Only compiled for `wasm32`. The store and the view description compile natively, so tests do not need a browser. |

## Why ConTime and minbin

The simple design is a `Vec` of events and JSON. Reagera used both of these libraries instead, because each one removes a class of code that the simple design makes you write.

ConTime replaces the event list and the state cache. With a `Vec`, a late event from another client means that you find its position, insert it, throw away the current state, and fold the whole list again. Every time-travel query is also a full fold from the start. ConTime does this work itself. It keeps the events in canonical order by `(time, id)`, so arrival order never changes the result. It ignores an event whose ID it has seen, so a client can resend without harm. It saves a checkpoint every N events, so a query at any time costs a fold from the nearest checkpoint, not from the start. A late event invalidates only the checkpoints after it. The devtools slider, `state_at`, and `export` are all queries against the same history, so there is no second copy that can drift from the first.

minbin replaces JSON and makes the record ID possible. A record's ID is the hash of its bytes. For that hash to be the same on every machine, the bytes must be the same on every machine. JSON does not promise this: field order, whitespace, and number formatting can differ between encoders. minbin encodes a value in exactly one way, because the schema you write with `minbin_enum!` fixes the tag of every variant and the width of every integer. So two stores compute the same ID for the same event, deduplication works across clients, and an imported session cannot hold a record whose ID disagrees with its content. The schema is also the contract of the app: a new variant gets a new tag, and an old session still decodes. The bytes are small. A todo session of a few dozen events is a few hundred bytes, which fits in a URL hash.

## A complete app

```rust
use reagera::{html::*, minbin, App, Ctx, Frame, Html, Nothing};

// 1. The wire format of every event. This is the contract of the app.
#[derive(Clone, Debug, Default, PartialEq)]
enum Msg { #[default] Reset, Add(i64) }

minbin::minbin_enum! { Msg [
    [0 => Self::Reset],
    [1 => Self::Add(n: i64)]
] }

// 2. The state and the update rule. Deterministic, no panics.
#[derive(Clone, Default)]
struct State { count: i64 }

struct Counter;

impl App for Counter {
    const NAME: &'static str = "counter/1";
    type Event = Msg;
    type State = State;
    type Effect = Nothing;   // this app asks nothing of the outside world
    type Outcome = Nothing;

    // `ctx` gives the record's time, origin and content ID, and `ctx.emit` sends a request.
    fn apply(state: &mut State, event: &Msg, ctx: &mut Ctx<Self>) {
        let _ = ctx;
        match event {
            Msg::Add(n) => state.count += n,
            Msg::Reset => state.count = 0,
        }
    }

    // 3. The view: a pure function of state.
    fn view(state: &State, frame: &Frame) -> Html<Msg> {
        div()
            .child(button().on_click(Msg::Add(-1)).text("-"))
            .child(span().text(format!(" {} ", state.count)))
            .child(button().on_click(Msg::Add(1)).text("+"))
            .child(button().on_click(Msg::Reset).text("reset"))
            .child_if(!frame.live, || em().text("viewing the past").build())
            .build()
    }
}

// 4. Mount it. The devtools panel is on by default.
#[wasm_bindgen::prelude::wasm_bindgen(start)]
fn start() {
    let rt = reagera::web::mount::<Counter>("root", reagera::web::MountOptions::default());
    std::mem::forget(rt);
}
```

There are no components with local state. State that is not in the log cannot be
replayed, so Reagera has no place for it. Keep a draft in the input element itself and
dispatch an event when the user commits it. The todo example keys its input by the next
id, so the element is recreated empty after every add.

## Records

| Field | Meaning |
| --- | --- |
| `ms` | Wall-clock milliseconds. ConTime orders by this first. |
| `seq` | Per-origin counter. Together with `ms` it forms the ConTime timestamp (`Tick`), so two events in the same millisecond keep their order. `dispatch` moves it past the newest record the store knows, so a reply to an event from another origin sorts after that event even in the same millisecond. |
| `origin` | Who produced the record. Each store gets a random name unless you set one. |
| `body` | Tag `u8`: 0 then your event, or 1 then a reply: the request's ID (`u128`), the request, and the outcome. |
| `id` | FNV-1a 128 of the four fields above, as minbin bytes. Used for deduplication and as the ConTime event ID. |

`store.dispatch(body)` records an event at the clock's time, but never earlier than the
newest record the store knows. So a store's own records keep the order they were made in
even if the clock jumps back, and a reply to something the store has seen is ordered
after it. `store.dispatch_at(body, ms)` sets the time explicitly.
`Options::default().clock(...)` injects a fake clock in tests.
`store.receive(record)` inserts a record made elsewhere; a record older than the newest
one is placed at its own tick and later state is rebuilt. The same records received in
any order produce the same state.

Decide identity in `apply`, never in the view. A view runs against the state of the
last render, and a second event before the next render sees the same state. The todo
example uses the record's content ID as the todo's id, so two adds in one frame cannot
collide.

## Time travel

```rust
store.state();            // state at the head
store.head_ms();          // the millisecond the UI shows
store.live();             // true when the head follows the newest event
store.backtrack(ms);      // show the state as of ms; nothing is deleted
store.go_live();          // follow the newest event again
store.state_at(ms);       // query any time without moving the head
store.events_between(a, b);   // what happened between two frames
```

Time travel is read-only. `dispatch` while the head is in the past returns
`StoreError::NotLive` and the devtools panel says so. Other stores may hold the same
history, so no store discards records. To branch, `store.fork(options)` gives an
independent store with a copy of the history, like an Aurora clone.

## Effects

A request to the outside world is not an event and not a state. It is the third kind
of thing in a UI, and the one that `useEffect` makes hard. Reagera gives it three
rules:

1. `apply` and `resolve` are pure. When they need the outside world, they call
   `ctx.emit(request)` and get back the request's ID. The request is not written to the
   log: it is derived from the record that emitted it, so every fold finds the same
   request with the same ID. The requests without an answer are part of the snapshot,
   next to the state. `store.pending()` lists them.
2. `perform` is the one method that may touch the outside world. The runtime calls it
   once per request, only for requests this store's origin emitted, and never during
   replay. It receives the request and a `done` callback to call later with the answer.
3. The answer is a record. `done(outcome)` calls `store.reply(id, outcome)`, which
   writes a `Reply` record that carries the request's ID, the request, and the answer.
   The fold meets it, removes the request from the pending list, and calls `resolve`.
   A second answer to the same request changes nothing.

```rust
#[derive(Clone, Debug, Default, PartialEq)]
enum Effect { #[default] Nothing, Charge { cents: u32 } }
minbin::minbin_enum! { Effect [[0 => Self::Nothing], [1 => Self::Charge { cents: u32 }]] }

#[derive(Clone, Debug, Default, PartialEq)]
enum Outcome { #[default] Nothing, Charged { receipt: String }, Declined { reason: String } }
minbin::minbin_enum! { Outcome [[0 => Self::Nothing], [1 => Self::Charged { receipt: String }], [2 => Self::Declined { reason: String }]] }

impl App for Checkout {
    type Effect = Effect;
    type Outcome = Outcome;

    fn apply(state: &mut State, event: &Msg, ctx: &mut Ctx<Self>) {
        match event {
            Msg::Pay if state.paying.is_none() => {
                state.paying = Some(ctx.emit(Effect::Charge { cents: state.total() }));
            }
            // ...
        }
    }

    fn resolve(state: &mut State, request: &Effect, outcome: &Outcome, ctx: &mut Ctx<Self>) {
        state.paying = None;
        match outcome {
            Outcome::Charged { receipt } => { state.receipts.push(receipt.clone()); state.cart.clear(); }
            Outcome::Declined { reason } => state.declined = Some(reason.clone()),
            Outcome::Nothing => {}
        }
    }

    // Browser only. Fetch, setTimeout, Math.random: anything, as long as it calls `done` once.
    fn perform(pending: &Pending<Effect>, done: Done<Outcome>) {
        gateway::charge(pending, done);
    }
    // ...
}
```

What this buys:

- No dependency array. A request is emitted by a state transition, not by a render.
  Two `Pay` events in one frame emit one charge if `apply` says so, and the log shows
  both events and the one request.
- No stale-response bug. `resolve` runs in log order with the state of that moment,
  and the record names the request it answers.
- Replay performs nothing. Import a session, and every request in it either has its
  answer in the log or stays pending. A pending request of another origin is shown in
  the devtools panel and never performed. A test can answer it by hand with
  `store.reply(id, outcome)`.
- A reply may emit a request of its own, so a chain of calls is a chain of records.

The clock and random numbers are effects too. A record already carries its time, so
most apps need neither. When one does, emit a request and record the answer.

In a test, `store.pending()` lists the requests, `store.reply(id, outcome)` answers one,
and `store.pending_at(ms)` shows what was in flight at any moment. A reply is accepted
while the head is in the past, because it settles something already in the history.

## Sessions

```rust
let bytes = store.export();              // Vec<u8>
let text = store.export_text();          // base64 of the same bytes
let copy = Store::<Counter>::import(&bytes, Options::default())?;
let same = Store::<Counter>::import_text(&text, Options::default())?;
```

Session layout, in minbin terms:

```text
u32   magic "RGRA"
u8    format version (3)
String app name, checked on import
bool  live
i64   head ms (0 when live)
u32   record count, then per record: i64 ms, String origin, u32 seq, u8 body tag,
             then the event in your enum's format (tag 0),
             or u128 request ID, the request, the outcome (tag 1)
```

IDs are not stored; they are recomputed on import, so a session cannot carry a record
whose ID disagrees with its content. Importing a session of another app fails before
any record is read.

To reproduce a bug: ask the reporter to press "export" in the devtools panel and paste
the text. In a test, call `Store::import_text` and assert on `store.state()`, or on
`store.state_at(ms)` for the moment they describe. The answers the outside world gave
are in the session, so the test does not need a network or a fake server. The todo example keeps the whole
session in the URL hash, so the URL itself is the bug report.

## Checkpoints

ConTime saves a checkpoint every `checkpoint_interval` events (default 64) and answers
a query from the nearest checkpoint at or before the requested time. A late event
invalidates only the checkpoints after its position. Reagera does not use ConTime's
pruning; UI sessions are small and an exported session must be complete.

## Testing views without a browser

`Html<M>` compiles natively. `to_html_string()` renders it, `find(tag)` locates an
element, and `fire(event, &DomEvent)` runs its listeners and returns the messages:

```rust
let html = Counter::view(store.state(), &store.frame());
assert_eq!(html.to_html_string(), "<div><button>-</button>...");
let msgs = html.find("button").unwrap().fire("click", &DomEvent::default());
assert_eq!(msgs, vec![Msg::Add(-1)]);
```

## Build and run the examples

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
./build.sh
python3 -m http.server 8080 --directory www
```

Open http://localhost:8080/. Tests: `cargo test --workspace`.

Checkout is the effects demo. Add items and press pay. The gateway answers after a
random delay and declines three times in ten. The panel lists the charge while it is
pending, and the answer as a reply record when it arrives. Press pay twice: one charge.
Export the session, open a new tab, import it: the same receipts and the same declines,
and the gateway is not called. Scrub back to a moment with a charge in flight: the page
says so, from the pending list of that moment.

Todo is the classic app. Add some todos, then drag the slider in the panel at the bottom
right. Press "export", copy the text, and paste it into the import box of a fresh tab to
replay the session. Or copy the URL.

Latency lab shows the part that makes Reagera different. Two clients, alice and bob,
share an arena. Each click is a record. Records reach the other client after a delay
with jitter that you control, so they often arrive out of order. A late record is drawn
with an orange ring and inserted behind newer ones. The number on each dot is the order
the history settles on. A status line turns green when alice, bob and a reference store
agree, which happens as soon as nothing is in flight, whatever the arrival order was.

## Deploy

The Dockerfile builds the examples and serves `www/` with nginx. `fly.toml` runs one
shared-cpu-1x machine that stops when idle and starts on the first request.

## Limits

- `contime` is pinned to a git revision because the crates.io release does not expose
  the `manual` feature yet.
- The renderer matches children by position. There is no keyed reordering, so a list
  that reorders often will recreate nodes. A different key forces a new node.
- The history lives in memory. Persist the export bytes yourself. The examples use the
  URL hash.
- `apply` must not panic; a panic inside ConTime's worker aborts the page.
- minbin caps a string at 100 KiB and a record at 1 MiB. `dispatch` refuses a larger
  event with `StoreError::TooLarge` and records nothing. A session may be 64 MiB.
- A request is performed by the origin that emitted it. A store that imports a session
  under a new origin performs nothing, which is what a replay wants. A reload of the
  same tab keeps its origin (the runtime stores it in `sessionStorage`), so the tab
  performs again the requests it left without an answer. Pass the request ID to the
  outside world as an idempotency key, so a request performed twice is charged once.
  An API without idempotency keys cannot be made safe this way: treat its answer as
  unknown and ask the user.
- Clocks of different clients are not reconciled. A client's own records are monotonic,
  and a reply is ordered after what the client has seen, but two clients that have not
  seen each other's records are ordered by their own clocks.

## License

MIT
