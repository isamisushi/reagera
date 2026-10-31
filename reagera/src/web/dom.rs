//! Diffing renderer: `Html<M>` in, DOM mutations out, messages back through a callback.
//!
//! Children are matched by position within their parent; a different tag or key
//! replaces the node. Listeners are attached once per element and event name and
//! only their handler is swapped on later renders, so the DOM listener list stays
//! stable across renders.

use crate::html::{DomEvent, Element as VElement, Handler, Html, Listener, Prop};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{Element, Event, Node};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const SVG_TAGS: &[&str] = &["svg", "circle", "rect", "line", "path", "g", "polyline", "polygon", "text", "ellipse"];

struct Slot<M> {
    event: &'static str,
    handler: Rc<RefCell<(Handler<M>, bool)>>,
    closure: Closure<dyn FnMut(Event)>,
    target: Element,
}

impl<M> Drop for Slot<M> {
    fn drop(&mut self) {
        let _ = self.target.remove_event_listener_with_callback(self.event, self.closure.as_ref().unchecked_ref());
    }
}

enum Kind<M> {
    Text(String),
    Element {
        tag: &'static str,
        key: Option<String>,
        attrs: Vec<(&'static str, String)>,
        props: Vec<Prop>,
        slots: Vec<Slot<M>>,
        children: Vec<Rendered<M>>,
    },
}

struct Rendered<M> {
    node: Node,
    kind: Kind<M>,
}

/// Renders `Html<M>` into a container and reports messages to `dispatch`.
pub struct Renderer<M> {
    container: Element,
    root: Option<Rendered<M>>,
    dispatch: Rc<dyn Fn(M)>,
}

impl<M: 'static> Renderer<M> {
    /// A renderer for `container`. The container is emptied.
    pub fn new(container: Element, dispatch: Rc<dyn Fn(M)>) -> Self {
        container.set_text_content(None);
        Self { container, root: None, dispatch }
    }

    /// Diff `tree` against the previous render and apply the difference.
    pub fn render(&mut self, tree: Html<M>) {
        match self.root.take() {
            None => {
                let r = self.create(tree);
                self.container.append_child(&r.node).expect("append");
                self.root = Some(r);
            }
            Some(prev) => {
                let r = self.patch(prev, tree);
                self.root = Some(r);
            }
        }
    }

    /// Empty the container.
    pub fn unmount(&mut self) {
        self.root = None;
        self.container.set_text_content(None);
    }

    fn create(&self, tree: Html<M>) -> Rendered<M> {
        let doc = super::document();
        match tree {
            Html::Text(t) => Rendered { node: doc.create_text_node(&t).into(), kind: Kind::Text(t) },
            Html::Element(v) => {
                let el = if SVG_TAGS.contains(&v.tag) {
                    doc.create_element_ns(Some(SVG_NS), v.tag).expect("createElementNS")
                } else {
                    doc.create_element(v.tag).expect("createElement")
                };
                for (k, val) in &v.attrs {
                    el.set_attribute(k, val).expect("setAttribute");
                }
                for p in &v.props {
                    set_prop(&el, p);
                }
                let slots = v.listeners.iter().map(|l| self.slot(&el, l)).collect();
                let children = v.children.into_iter().map(|c| self.create(c)).collect::<Vec<_>>();
                for c in &children {
                    el.append_child(&c.node).expect("append");
                }
                Rendered { node: el.clone().into(), kind: Kind::Element { tag: v.tag, key: v.key, attrs: v.attrs, props: v.props, slots, children } }
            }
        }
    }

    fn patch(&self, prev: Rendered<M>, next: Html<M>) -> Rendered<M> {
        match (prev, next) {
            (Rendered { node, kind: Kind::Text(old) }, Html::Text(new)) => {
                if old != new {
                    node.set_text_content(Some(&new));
                }
                Rendered { node, kind: Kind::Text(new) }
            }
            (Rendered { node, kind: Kind::Element { tag, key, attrs, props, slots, children } }, Html::Element(v)) if tag == v.tag && key == v.key => {
                let el: Element = node.clone().unchecked_into();
                self.patch_element(el, node, tag, attrs, props, slots, children, v)
            }
            (prev, next) => {
                let fresh = self.create(next);
                if let Some(parent) = prev.node.parent_node() {
                    parent.replace_child(&fresh.node, &prev.node).expect("replaceChild");
                }
                fresh
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn patch_element(
        &self,
        el: Element,
        node: Node,
        tag: &'static str,
        old_attrs: Vec<(&'static str, String)>,
        old_props: Vec<Prop>,
        old_slots: Vec<Slot<M>>,
        old_children: Vec<Rendered<M>>,
        v: VElement<M>,
    ) -> Rendered<M> {
        for (k, _) in &old_attrs {
            if !v.attrs.iter().any(|(nk, _)| nk == k) {
                el.remove_attribute(k).expect("removeAttribute");
            }
        }
        for (k, val) in &v.attrs {
            if old_attrs.iter().find(|(ok, _)| ok == k).map(|(_, ov)| ov) != Some(val) {
                el.set_attribute(k, val).expect("setAttribute");
            }
        }
        for p in &v.props {
            set_prop(&el, p);
        }
        let _ = old_props;
        // Reuse a slot per event name; swap its handler. Extra old slots drop and detach.
        let mut old_slots = old_slots;
        let mut slots = Vec::with_capacity(v.listeners.len());
        for l in &v.listeners {
            if let Some(i) = old_slots.iter().position(|s| s.event == l.event) {
                let s = old_slots.swap_remove(i);
                *s.handler.borrow_mut() = (l.handler.clone(), l.prevent_default);
                slots.push(s);
            } else {
                slots.push(self.slot(&el, l));
            }
        }
        drop(old_slots);

        let mut old_children = old_children.into_iter();
        let mut new_children = v.children.into_iter();
        let mut children = Vec::new();
        loop {
            match (old_children.next(), new_children.next()) {
                (Some(o), Some(n)) => children.push(self.patch(o, n)),
                (None, Some(n)) => {
                    let r = self.create(n);
                    el.append_child(&r.node).expect("append");
                    children.push(r);
                }
                (Some(o), None) => {
                    el.remove_child(&o.node).expect("removeChild");
                }
                (None, None) => break,
            }
        }
        Rendered { node, kind: Kind::Element { tag, key: v.key, attrs: v.attrs, props: v.props, slots, children } }
    }

    fn slot(&self, el: &Element, l: &Listener<M>) -> Slot<M> {
        let handler = Rc::new(RefCell::new((l.handler.clone(), l.prevent_default)));
        let h = handler.clone();
        let dispatch = self.dispatch.clone();
        let target = el.clone();
        let closure = Closure::<dyn FnMut(Event)>::new(move |ev: Event| {
            let (handler, prevent) = { let g = h.borrow(); (g.0.clone(), g.1) };
            if prevent {
                ev.prevent_default();
            }
            let dom_event = dom_event(&ev, &target);
            if let Some(msg) = handler(&dom_event) {
                dispatch(msg);
            }
        });
        el.add_event_listener_with_callback(l.event, closure.as_ref().unchecked_ref()).expect("addEventListener");
        Slot { event: l.event, handler, closure, target: el.clone() }
    }
}

fn set_prop(el: &Element, p: &Prop) {
    match p {
        Prop::Value(v) => {
            if let Some(i) = el.dyn_ref::<web_sys::HtmlInputElement>() {
                if &i.value() != v {
                    i.set_value(v);
                }
            } else if let Some(t) = el.dyn_ref::<web_sys::HtmlTextAreaElement>() {
                if &t.value() != v {
                    t.set_value(v);
                }
            } else if let Some(s) = el.dyn_ref::<web_sys::HtmlSelectElement>() {
                if &s.value() != v {
                    s.set_value(v);
                }
            }
        }
        Prop::Checked(c) => {
            if let Some(i) = el.dyn_ref::<web_sys::HtmlInputElement>() {
                if i.checked() != *c {
                    i.set_checked(*c);
                }
            }
        }
    }
}

fn dom_event(ev: &Event, current: &Element) -> DomEvent {
    let mut out = DomEvent::default();
    if let Some(t) = ev.target() {
        if let Some(i) = t.dyn_ref::<web_sys::HtmlInputElement>() {
            out.value = i.value();
            out.checked = i.checked();
        } else if let Some(t) = t.dyn_ref::<web_sys::HtmlTextAreaElement>() {
            out.value = t.value();
        } else if let Some(s) = t.dyn_ref::<web_sys::HtmlSelectElement>() {
            out.value = s.value();
        }
    }
    if let Some(k) = ev.dyn_ref::<web_sys::KeyboardEvent>() {
        out.key = k.key();
    }
    if let Some(m) = ev.dyn_ref::<web_sys::MouseEvent>() {
        let rect = current.get_bounding_client_rect();
        out.x = (f64::from(m.client_x()) - rect.left()) as i32;
        out.y = (f64::from(m.client_y()) - rect.top()) as i32;
        out.width = rect.width() as i32;
        out.height = rect.height() as i32;
    }
    out
}
