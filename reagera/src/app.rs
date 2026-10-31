//! The contract an application implements.

use crate::{
    effect::{Ctx, Done, Pending},
    html::Html,
};
use minbin::ToFromBytes;
use std::fmt::Debug;

/// Anything that can go on the wire with minbin. Implemented automatically.
pub trait Wire: for<'a> ToFromBytes<'a> + Default + Clone + Debug + 'static {}
impl<T> Wire for T where T: for<'a> ToFromBytes<'a> + Default + Clone + Debug + 'static {}

/// What the view is allowed to know about the store besides the state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// True when the head follows the newest event.
    pub live: bool,
    /// The millisecond the state is shown at.
    pub head_ms: i64,
    /// Number of retained events.
    pub events: usize,
    /// Number of requests without an answer at the head.
    pub pending: usize,
}

/// An application: events, a state, a pure update rule, requests to the outside
/// world and their answers, one place that performs them, and a view.
///
/// Four methods, and only one of them may touch the outside world:
///
/// | Method | Runs | Pure |
/// | --- | --- | --- |
/// | `apply` | on every event, in every fold of the history | yes |
/// | `resolve` | on every reply, in every fold of the history | yes |
/// | `perform` | once per request, in the browser, never on replay | no |
/// | `view` | on every render | yes |
pub trait App: Sized + 'static {
    /// Stable name, written into sessions so a wrong import is refused early.
    /// Change it when the wire format changes, for example `"todo/2"`.
    const NAME: &'static str;
    /// The event body. Implement `ToFromBytes` by hand or with `minbin_enum!`.
    type Event: Wire;
    /// The state. `Default` is the state before the first event.
    type State: Clone + Default + 'static;
    /// A request to the outside world. [`Nothing`](crate::Nothing) when the app has none.
    type Effect: Wire;
    /// The outside world's answer to a request. [`Nothing`](crate::Nothing) when the app has none.
    type Outcome: Wire;

    /// Change the state for one event. Must be deterministic and must not panic.
    /// `ctx` gives the record's time, origin and content ID, and `ctx.emit`
    /// sends a request to the outside world.
    fn apply(state: &mut Self::State, event: &Self::Event, ctx: &mut Ctx<Self>);

    /// Change the state for one answer. Same rules as `apply`. Called once per
    /// request: a second answer to the same request is ignored by the fold.
    /// The default does nothing.
    fn resolve(state: &mut Self::State, request: &Self::Effect, outcome: &Self::Outcome, ctx: &mut Ctx<Self>) {
        let _ = (state, request, outcome, ctx);
    }

    /// Carry out one request and call `done` with the answer, once, at any
    /// later time. This is the only method that may touch the outside world.
    /// The runtime calls it once per request and only for requests this
    /// store's origin emitted. It is never called during replay.
    /// The default does nothing, so the request stays pending.
    fn perform(pending: &Pending<Self::Effect>, done: Done<Self::Outcome>) {
        let _ = (pending, done);
    }

    /// Describe the page for a state. Pure.
    fn view(state: &Self::State, frame: &Frame) -> Html<Self::Event>;
}
