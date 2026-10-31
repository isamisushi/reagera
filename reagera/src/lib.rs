//! Reagera: a front-end framework where the event log is the state.
//!
//! Every user action is a [`Record`]: a timestamped event with a stable content
//! ID. Records live in a ConTime history, so the state at any time is a
//! deterministic fold over the records at or before that time. Records are
//! encoded with minbin, so a whole session is a small, explicit byte string that
//! reproduces the same states anywhere.
//!
//! The crate has three layers:
//! - [`Store`]: the history, the movable head, sessions. Native and wasm.
//! - [`html`]: a virtual DOM description. Native and wasm, so views can be tested.
//! - [`web`]: the renderer, the runtime and the devtools panel. wasm32 only.
#![deny(missing_docs)]

pub mod app;
pub mod base64;
pub mod hash;
pub mod html;
pub mod record;
pub mod session;
pub mod store;
pub mod tick;

#[cfg(target_arch = "wasm32")]
pub mod web;

pub use app::{App, Frame, Wire};
pub use html::{DomEvent, Element, Handler, Html};
pub use record::Record;
pub use store::{Options, Store, StoreError};
pub use tick::Tick;

/// Re-exported so apps only need one dependency for their event types.
pub use minbin;
