//! The contract an application implements.

use crate::html::Html;
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
}

/// An application: an event type with a wire format, a state, a pure update rule and a view.
pub trait App: 'static {
    /// Stable name, written into sessions so a wrong import is refused early.
    /// Change it when the event format changes, for example `"todo/2"`.
    const NAME: &'static str;
    /// The event body. Implement `ToFromBytes` by hand or with `minbin_enum!`.
    type Event: Wire;
    /// The state. `Default` is the state before the first event.
    type State: Clone + Default + 'static;

    /// Change the state for one event. Must be deterministic and must not panic.
    fn apply(state: &mut Self::State, event: &Self::Event, at_ms: i64);

    /// Describe the page for a state. Pure.
    fn view(state: &Self::State, frame: &Frame) -> Html<Self::Event>;
}
