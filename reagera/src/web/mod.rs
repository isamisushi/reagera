//! Browser side: the DOM renderer, the runtime and the devtools panel.

mod devtools;
mod dom;
mod runtime;

pub use devtools::{DevMsg, DevState};
pub use dom::Renderer;
pub use runtime::{mount, mount_store, MountOptions, Runtime};

use wasm_bindgen::prelude::*;

/// `window.document`.
pub fn document() -> web_sys::Document {
    web_sys::window().expect("window").document().expect("document")
}

/// Milliseconds since the Unix epoch, from `Date.now()`.
pub fn now_ms() -> i64 {
    js_sys::Date::now() as i64
}

/// Schedule `f` on the next animation frame.
pub fn next_frame(f: impl FnOnce() + 'static) {
    let closure = Closure::once_into_js(f);
    web_sys::window()
        .expect("window")
        .request_animation_frame(closure.unchecked_ref())
        .expect("requestAnimationFrame");
}

/// Call `f` every `ms` milliseconds. Returns the interval handle.
pub fn every(ms: i32, f: impl FnMut() + 'static) -> i32 {
    let closure = Closure::<dyn FnMut()>::new(f);
    let id = web_sys::window()
        .expect("window")
        .set_interval_with_callback_and_timeout_and_arguments_0(closure.as_ref().unchecked_ref(), ms)
        .expect("setInterval");
    closure.forget();
    id
}

/// Call `f` once after `ms` milliseconds.
pub fn after(ms: i32, f: impl FnOnce() + 'static) {
    let closure = Closure::once_into_js(f);
    web_sys::window()
        .expect("window")
        .set_timeout_with_callback_and_timeout_and_arguments_0(closure.unchecked_ref(), ms)
        .expect("setTimeout");
}

/// A number in `[0, 1)` from `Math.random()`. Not deterministic: record what you do with it.
pub fn random() -> f64 {
    js_sys::Math::random()
}

/// Log to the browser console.
pub fn log(s: &str) {
    web_sys::console::log_1(&JsValue::from_str(s));
}
