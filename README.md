# Reagera

A front-end framework where the event log is the state. Rust, compiled to WebAssembly.

Reagera (Swedish for "react") records every user action as a timestamped event in a
[ConTime](https://github.com/humacss/contime) history and encodes it with
[minbin](https://github.com/humacss/minbin). The state you render is a fold over that
history. Nothing else is the source of truth. This is the idea behind Amazon Aurora,
where storage is a redo log and every page is derived from it. Reagera applies it to a UI.

Because the log is the state, three things come for free:

- Query any moment. `store.state_at(ms)` returns the state at that millisecond. The
  devtools panel has a slider that scrubs through the session.
- Reproduce any session. `store.export()` returns a few hundred bytes. `Store::import`
  rebuilds the identical history on another machine, in a test, or from a bug report.
- Accept events late or out of order. ConTime orders events by their own time, never
  by arrival. A late event lands at its proper place and later state is rebuilt.

Live examples: https://reagera.fly.dev/

## What each dependency does

| Crate | Role in Reagera |
| --- | --- |
| contime | Keeps the history. Canonical ordering by `(time, id)`, deduplication, checkpoints every N events, point-in-time queries, late insertion with rebuild. Reagera uses the `manual` driver, so there are no threads: the worker is stepped on the browser main thread until it is idle. |
| minbin | Encodes records and sessions. Fixed-width big-endian integers, length-prefixed strings, explicit enum tags. You write the schema with `minbin_enum!` or by hand; the bytes follow it. |
| wasm-bindgen, web-sys | The DOM renderer, the runtime and the devtools panel. Only compiled for `wasm32`. The store and the view description compile natively, so tests do not need a browser. |

## A complete app

```rust
use reagera::{html::*, minbin, App, Frame, Html};

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

    fn apply(state: &mut State, event: &Msg, _at_ms: i64) {
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
| `seq` | Per-origin counter. Together with `ms` it forms the ConTime timestamp (`Tick`), so two events in the same millisecond keep their order. |
| `origin` | Who produced the record. Each store gets a random name unless you set one. |
| `body` | Your event. |
| `id` | FNV-1a 128 of the four fields above, as minbin bytes. Used for deduplication and as the ConTime event ID. |

`store.dispatch(body)` records an event at the clock's time. `store.dispatch_at(body, Some(ms))`
sets the time. `Options::default().clock(...)` injects a fake clock in tests.
`store.receive(record)` inserts a record made elsewhere; a record older than the newest
one is placed at its own tick and later state is rebuilt. The same records received in
any order produce the same state.

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

Dispatching while the head is in the past drops every event after the head and starts
a new future, as a rewind does in an editor. Use `store.fork(options)` first to keep
both branches. A fork is an independent store with a copy of the history, like an
Aurora clone.

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
u8    format version (2)
String app name, checked on import
bool  live
i64   head ms (0 when live)
Vec<Record>: per record i64 ms, String origin, u32 seq, then the body in your enum's format
```

IDs are not stored; they are recomputed on import, so a session cannot carry a record
whose ID disagrees with its content. Importing a session of another app fails before
any record is read.

To reproduce a bug: ask the reporter to press "export" in the devtools panel and paste
the text. In a test, call `Store::import_text` and assert on `store.state()`, or on
`store.state_at(ms)` for the moment they describe. The todo example keeps the whole
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

## License

MIT
