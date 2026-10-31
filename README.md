# Reagera

A front-end framework where the event log is the state. Rust, compiled to WebAssembly.

Reagera records every user action as a timestamped event in a
[ConTime](https://github.com/humacss/contime) history. It encodes each event with
[minbin](https://github.com/humacss/minbin). The state you render is a fold (a replay
of every event in order) over that history. Nothing else is the source of truth.

Because the log is the state, you get four things:

- Any moment. `store.state_at(ms)` returns the state at that millisecond.
- Any session. `store.export()` is a few hundred bytes that rebuild the same history anywhere.
- Any order. A late event lands at its own time, and the states after it are rebuilt.
- Any side effect, once. The app emits a request, the runtime performs it once, and the
  answer is a record. A replay reads the recorded answer and performs nothing.

Live examples: https://reagera.fly.dev/

## Reagera and React

| Topic | React | Reagera |
| --- | --- | --- |
| Source of truth | Current state in components or a store | An ordered log of timestamped events |
| Local state | `useState` in any component | None. All state is in the log. |
| Past states | Not kept | `state_at(ms)` and `backtrack(ms)` |
| Order of updates | Arrival order | The time of each event, whatever the arrival order |
| Late or duplicate events | Application code handles them | The history inserts late events by time and ignores duplicates |
| Side effects | `useEffect`, run from the render, ordered by a dependency array | `apply` emits a request. The answer is a record. `perform` is the one place that touches the outside world. |
| A stale response | A ref or an abort signal | `resolve` sees the request and the state of that moment, in log order |
| A bug report | Steps to reproduce | The session text. Import it in a test. |
| Testing | A DOM, usually jsdom | Views compile natively. No DOM. |
| Language | JavaScript or TypeScript | Rust, compiled to WebAssembly |

If you need its ecosystem, server-side rendering, keyed list reordering, or a small
download, use React. Each Reagera example is about 100 KB of WebAssembly after gzip.
If exact replay of a session matters more, use Reagera: collaborative tools,
offline-first apps, or bugs that you cannot reproduce. Reagera is an early project with
three examples and no production use.

## ConTime and minbin

ConTime is the history. It keeps events in canonical order by `(time, id)`, ignores an
event whose ID it has seen, and saves a checkpoint every 64 events. A query at any time
folds from the nearest checkpoint. A late event invalidates only the checkpoints after
it. Reagera uses the `manual` driver, so there are no threads: the browser main thread
steps the worker until it is idle. The devtools slider, `state_at`, and `export` are
all queries against this one history.

minbin encodes records. Fixed-width integers, length-prefixed strings, and explicit enum
tags mean that a value has exactly one encoding on every machine. A record's ID is the
FNV-1a 128 hash of its bytes, so two stores compute the same ID for the same event. An
imported session cannot carry a record whose ID disagrees with its content.

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

    // `ctx` gives the record's time, origin and content ID. `ctx.emit` sends a request.
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

There is no component-local state, because state outside the log cannot be replayed.
Keep a draft in the input element. When the user commits it, dispatch an event.

Decide identity in `apply`, never in the view. The view runs against the state of the
last render, so two events in one frame see the same state. The todo example uses the
record's content ID as the todo's id.

## Effects

A request to the outside world is neither an event nor a state. Reagera gives it three
rules:

1. `apply` and `resolve` are pure. To reach the outside world, they call
   `ctx.emit(request)` and get the request's ID back. The request is not written to the
   log. It is derived from the record that emitted it, so every fold finds the same
   request with the same ID. Requests without an answer are part of the snapshot, next
   to the state. `store.pending()` lists them.
2. `perform` is the one method that touches the outside world. The runtime calls it once
   per request, only for requests that this store's origin emitted, and never during
   replay. It gets the request and a `done` callback for the answer.
3. The answer is a record. `done(outcome)` calls `store.reply(id, outcome)`, which
   writes a `Reply` record with the request's ID, the request, and the answer. The fold
   removes the request from the pending list and calls `resolve`. A second answer to
   the same request changes nothing. A reply can emit a request of its own.

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
            // One charge at a time. A second Pay in the same frame emits nothing.
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

    // Browser only. Fetch, setTimeout, Math.random: anything that calls `done` once.
    fn perform(pending: &Pending<Effect>, done: Done<Outcome>) {
        gateway::charge(pending, done);
    }
    // ...
}
```

The clock and random numbers are effects too. A record already carries its time, so
most apps need neither. In a test, `store.pending()` lists the requests,
`store.reply(id, outcome)` answers one by hand, and `store.pending_at(ms)` shows what
was in flight at any moment.

## Records and sessions

| Field | Meaning |
| --- | --- |
| `ms` | Wall-clock milliseconds. ConTime orders by this first. |
| `seq` | Per-origin counter. With `ms` it forms the ConTime timestamp, so events in one millisecond keep their order. |
| `origin` | The store that made the record. Random unless you set it. |
| `body` | Tag `u8`: 0 then your event, or 1 then a reply: request ID (`u128`), request, outcome. |
| `id` | FNV-1a 128 of the fields above as minbin bytes. Not stored. Recomputed on import. |

`store.dispatch(event)` records at the clock's time, but never earlier than the newest
record the store knows. When the clock jumps back, a store's own records keep their
order. A reply sorts after what the store has seen. `store.receive(record)` inserts
a record made elsewhere. The same records in any order give the same state.

```rust
let bytes = store.export();              // Vec<u8>
let text = store.export_text();          // base64 of the same bytes
let copy = Store::<Counter>::import(&bytes, Options::default())?;
```

Session layout:

```text
u32    magic "RGRA"
u8     format version (3)
String app name, refused on import when it differs
bool   live
i64    head ms (0 when live)
u32    record count, then each record: i64 ms, String origin, u32 seq, body
```

To reproduce a bug, ask the reporter to press "export" in the devtools panel and send
the text. In a test, call `Store::import_text` and assert on `store.state()` or
`store.state_at(ms)`. The answers from the outside world are in the session, so the test
needs no network and no fake server.

## Time travel

```rust
store.backtrack(ms);          // show the state as of ms; nothing is deleted
store.go_live();              // follow the newest event again
store.state_at(ms);           // query any time without moving the head
store.events_between(a, b);   // what happened between two moments
store.fork(options);          // an independent store with a copy of the history
```

Time travel is read-only. `dispatch` while the head is in the past returns
`StoreError::NotLive`. A `reply` is accepted, because it settles something already in
the history.

## Testing views without a browser

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

Open http://localhost:8080/. Run the tests with `cargo test --workspace`.

- Compare: the same todo app in React 18 and in Reagera, side by side. A script bar sends
  the same action to both. Under each app, a panel shows what its framework keeps: React
  keeps the latest reducer result, Reagera keeps the log, the fold, and a head to move.
- Todo: the classic app. The session lives in the URL hash, so the URL is the bug report.
- Latency lab: two clients, delayed delivery with jitter. A late record gets an orange
  ring. Both clients agree as soon as nothing is in flight.
- Checkout: a slow payment gateway that declines three times in ten. Press pay twice:
  one charge. Import the session in a new tab: the same receipts, and no call to the gateway.

The Dockerfile builds the examples and serves `www/` with nginx. `fly.toml` runs one
machine. When the machine is idle, it stops.

## Limits

- `contime` is pinned to a git revision because the crates.io release does not expose
  the `manual` feature yet.
- The renderer matches children by position. A different key forces a new node.
- The history lives in memory. Persist the export bytes yourself.
- `apply` must not panic. A panic inside the ConTime worker aborts the page.
- minbin caps a string at 100 KiB and a record at 1 MiB. `dispatch` refuses a larger
  event with `StoreError::TooLarge`. A session can be 64 MiB.
- A request is performed by the origin that emitted it. A reload keeps the tab's origin
  in `sessionStorage`, so the tab performs again the requests it left without an answer.
  Pass the request ID to the outside world as an idempotency key (a token that makes a
  repeated call count once). Without such a key, treat the answer as unknown.
- Clocks of different clients are not reconciled. Two clients that have not seen each
  other's records are ordered by their own clocks.

## License

MIT
