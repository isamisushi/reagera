//! The runtime: owns the store and the renderer, turns messages into records,
//! performs pending requests once, records their answers, and re-renders on
//! the next animation frame.

use super::{devtools, dom::Renderer, DevMsg, DevState};
use crate::{App, Options, Store};
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    collections::HashSet,
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
    /// Origin name for records made here. Default: random, but kept in
    /// `sessionStorage`, so a reload of the same tab keeps its origin and
    /// performs the requests it left without an answer. Another tab, or a
    /// session pasted by someone else, gets a new origin and performs nothing.
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
    /// Requests handed to `A::perform`. Not part of the log: a reload starts
    /// with a new origin, and a request of another origin is never performed here.
    started: RefCell<HashSet<u128>>,
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
    let origin = options.origin.clone().or_else(|| tab_origin(A::NAME));
    let store_options = Options { origin, ..Options::default() };
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

/// The origin this tab used before, from `sessionStorage`, or none.
/// A new origin is written by `mount_store` once the store has chosen one.
fn tab_origin(app: &str) -> Option<String> {
    let storage = web_sys::window()?.session_storage().ok()??;
    storage.get_item(&format!("reagera:origin:{app}")).ok()?
}

fn remember_tab_origin(app: &str, origin: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.session_storage().ok().flatten()) {
        let _ = storage.set_item(&format!("reagera:origin:{app}"), origin);
    }
}

/// Mount an existing store.
pub fn mount_store<A: App>(root_id: &str, store: Store<A>, options: MountOptions) -> Runtime<A> {
    let doc = super::document();
    let root = doc.get_element_by_id(root_id).unwrap_or_else(|| panic!("no element with id {root_id:?}"));
    remember_tab_origin(A::NAME, store.origin());
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
        Inner { store: RefCell::new(store), view: RefCell::new(Renderer::new(root, dispatch)), dev: RefCell::new(dev), scheduled: Cell::new(false), started: RefCell::new(HashSet::new()), options }
    });
    let rt = Runtime { inner };
    rt.render_now();
    rt
}

impl<A: App> Runtime<A> {
    /// Record an event and schedule a render. While the head is in the past the
    /// message is dropped and a line is logged: time travel is read-only.
    pub fn dispatch(&self, msg: A::Event) {
        if let Err(e) = self.inner.store.borrow_mut().dispatch(msg) {
            super::log(&format!("reagera: {e}"));
        }
        self.schedule();
    }

    /// Record the answer to a request and schedule a render. Works while the
    /// head is in the past. An answer to a request that is not pending is
    /// dropped and a line is logged.
    pub fn reply(&self, effect: u128, outcome: A::Outcome) {
        if let Err(e) = self.inner.store.borrow_mut().reply(effect, outcome) {
            super::log(&format!("reagera: {e}"));
        }
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

    /// Render right now. The store is released before the DOM changes, because
    /// a DOM change can fire a listener synchronously, and that listener may dispatch.
    pub fn render_now(&self) {
        let (tree, dev_tree, hash) = {
            let mut store = self.inner.store.borrow_mut();
            let tree = A::view(store.state(), &store.frame());
            let dev_tree = self.inner.dev.borrow().as_ref().map(|(_, local)| devtools::view(&mut store, local));
            let hash = self.inner.options.session_in_url.then(|| store.export_text());
            (tree, dev_tree, hash)
        };
        self.inner.view.borrow_mut().render(tree);
        if let (Some(tree), Some((renderer, _))) = (dev_tree, self.inner.dev.borrow_mut().as_mut()) {
            renderer.render(tree);
        }
        if let (Some(hash), Some(w)) = (hash, web_sys::window()) {
            let _ = w.history().and_then(|h| h.replace_state_with_url(&JsValue::NULL, "", Some(&format!("#{hash}"))));
        }
        self.perform_pending();
    }

    /// Hand every request this origin emitted and has not started yet to
    /// `A::perform`. The store is released first, because `perform` may call
    /// `done` synchronously, and `done` writes a record.
    fn perform_pending(&self) {
        let fresh: Vec<_> = {
            let mut store = self.inner.store.borrow_mut();
            let mine = store.origin().to_string();
            let mut started = self.inner.started.borrow_mut();
            store.pending_live().into_iter().filter(|p| p.origin == mine && started.insert(p.id)).collect()
        };
        for pending in fresh {
            let rt = self.clone();
            let id = pending.id;
            A::perform(&pending, Box::new(move |outcome| rt.reply(id, outcome)));
        }
    }
}
