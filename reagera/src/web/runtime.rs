//! The runtime: owns the store and the renderer, turns messages into records,
//! re-renders on the next animation frame.

use super::{devtools, dom::Renderer, DevMsg, DevState};
use crate::{App, Options, Store};
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    rc::{Rc, Weak},
};
use wasm_bindgen::JsValue;

/// How to mount an app.
#[derive(Clone, Debug)]
pub struct MountOptions {
    /// Show the devtools panel.
    pub devtools: bool,
    /// Keep the session in the URL hash, and start from it when present.
    pub session_in_url: bool,
    /// Origin name for records made here. Default: random.
    pub origin: Option<String>,
}

impl Default for MountOptions {
    fn default() -> Self {
        Self { devtools: true, session_in_url: false, origin: None }
    }
}

struct Inner<A: App> {
    store: RefCell<Store<A>>,
    view: RefCell<Renderer<A::Event>>,
    dev: RefCell<Option<(Renderer<DevMsg>, DevState)>>,
    scheduled: Cell<bool>,
    options: MountOptions,
}

/// A mounted app.
pub struct Runtime<A: App> {
    inner: Rc<Inner<A>>,
}

impl<A: App> Clone for Runtime<A> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

/// Mount `A` into the element with id `root_id`.
pub fn mount<A: App>(root_id: &str, options: MountOptions) -> Runtime<A> {
    let store_options = Options { origin: options.origin.clone(), ..Options::default() };
    let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
    let store = match (options.session_in_url, hash.strip_prefix('#')) {
        (true, Some(text)) if !text.is_empty() => Store::<A>::import_text(text, store_options).unwrap_or_else(|e| {
            super::log(&format!("reagera: ignoring session in URL: {e}"));
            Store::new(Options::default())
        }),
        _ => Store::new(store_options),
    };
    mount_store(root_id, store, options)
}

/// Mount an existing store.
pub fn mount_store<A: App>(root_id: &str, store: Store<A>, options: MountOptions) -> Runtime<A> {
    let doc = super::document();
    let root = doc.get_element_by_id(root_id).unwrap_or_else(|| panic!("no element with id {root_id:?}"));
    let inner: Rc<Inner<A>> = Rc::new_cyclic(|weak: &Weak<Inner<A>>| {
        let w = weak.clone();
        let dispatch: Rc<dyn Fn(A::Event)> = Rc::new(move |msg| {
            if let Some(inner) = w.upgrade() {
                Runtime { inner }.dispatch(msg);
            }
        });
        let dev = if options.devtools {
            let panel = doc.create_element("div").expect("div");
            doc.body().expect("body").append_child(&panel).expect("append");
            let w = weak.clone();
            let dispatch: Rc<dyn Fn(DevMsg)> = Rc::new(move |msg| {
                if let Some(inner) = w.upgrade() {
                    Runtime { inner }.devtools(msg);
                }
            });
            Some((Renderer::new(panel, dispatch), DevState::default()))
        } else {
            None
        };
        Inner { store: RefCell::new(store), view: RefCell::new(Renderer::new(root, dispatch)), dev: RefCell::new(dev), scheduled: Cell::new(false), options }
    });
    let rt = Runtime { inner };
    rt.render_now();
    rt
}

impl<A: App> Runtime<A> {
    /// Record an event and schedule a render.
    pub fn dispatch(&self, msg: A::Event) {
        self.inner.store.borrow_mut().dispatch(msg);
        self.schedule();
    }

    /// Apply a devtools message and schedule a render.
    pub fn devtools(&self, msg: DevMsg) {
        if let Some((_, local)) = self.inner.dev.borrow_mut().as_mut() {
            devtools::handle(&mut self.inner.store.borrow_mut(), local, msg);
        }
        self.schedule();
    }

    /// Read the store.
    pub fn store(&self) -> Ref<'_, Store<A>> {
        self.inner.store.borrow()
    }

    /// Change the store directly, for example to `receive` records from a network. Call `schedule` after.
    pub fn store_mut(&self) -> RefMut<'_, Store<A>> {
        self.inner.store.borrow_mut()
    }

    /// Render on the next animation frame. Several calls coalesce into one render.
    pub fn schedule(&self) {
        if self.inner.scheduled.replace(true) {
            return;
        }
        let rt = self.clone();
        super::next_frame(move || {
            rt.inner.scheduled.set(false);
            rt.render_now();
        });
    }

    /// Render right now.
    pub fn render_now(&self) {
        let store = self.inner.store.borrow();
        let tree = A::view(store.state(), &store.frame());
        self.inner.view.borrow_mut().render(tree);
        if let Some((renderer, local)) = self.inner.dev.borrow_mut().as_mut() {
            renderer.render(devtools::view(&store, local));
        }
        if self.inner.options.session_in_url {
            if let Some(w) = web_sys::window() {
                let _ = w.history().and_then(|h| h.replace_state_with_url(&JsValue::NULL, "", Some(&format!("#{}", store.export_text()))));
            }
        }
    }
}
