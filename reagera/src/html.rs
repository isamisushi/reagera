//! A virtual DOM description, usable natively (tests) and in the browser.
//!
//! A view is a pure function from state to `Html<M>`. Listeners map a
//! [`DomEvent`] to an optional message `M`; the runtime dispatches the message
//! into the store. There is no component-local state: state that is not in the
//! log cannot be replayed, so there is nowhere to put it.

use std::rc::Rc;

/// What a listener may read from a browser event. Portable, so views are testable.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DomEvent {
    /// `value` of the input, textarea or select that raised the event.
    pub value: String,
    /// `checked` of the checkbox that raised the event.
    pub checked: bool,
    /// `key` of a keyboard event.
    pub key: String,
    /// Pointer x, relative to the element the listener is on.
    pub x: i32,
    /// Pointer y, relative to the element the listener is on.
    pub y: i32,
    /// Width of the element the listener is on.
    pub width: i32,
    /// Height of the element the listener is on.
    pub height: i32,
}

/// A listener. Return `None` to ignore the event.
pub type Handler<M> = Rc<dyn Fn(&DomEvent) -> Option<M>>;

/// A listener entry on an element.
#[derive(Clone)]
pub struct Listener<M> {
    /// DOM event name, for example `click`.
    pub event: &'static str,
    /// The handler.
    pub handler: Handler<M>,
    /// Call `preventDefault()` before the handler.
    pub prevent_default: bool,
}

/// A property set on the element object rather than as an attribute.
#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    /// `value`
    Value(String),
    /// `checked`
    Checked(bool),
}

/// An element node.
#[derive(Clone)]
pub struct Element<M> {
    /// Tag name.
    pub tag: &'static str,
    /// Identity for the position-based diff: a different key forces a new node.
    pub key: Option<String>,
    /// Attributes in insertion order.
    pub attrs: Vec<(&'static str, String)>,
    /// Properties.
    pub props: Vec<Prop>,
    /// Listeners.
    pub listeners: Vec<Listener<M>>,
    /// Children.
    pub children: Vec<Html<M>>,
}

/// A node.
#[derive(Clone)]
pub enum Html<M> {
    /// A text node.
    Text(String),
    /// An element.
    Element(Element<M>),
}

/// A text node.
pub fn text<M>(s: impl Into<String>) -> Html<M> {
    Html::Text(s.into())
}

/// An element with no attributes or children.
pub fn el<M>(tag: &'static str) -> Element<M> {
    Element { tag, key: None, attrs: Vec::new(), props: Vec::new(), listeners: Vec::new(), children: Vec::new() }
}

macro_rules! tags {
    ($($name:ident)*) => {
        $(
            #[doc = concat!("An empty `<", stringify!($name), ">`.")]
            pub fn $name<M>() -> Element<M> { el(stringify!($name)) }
        )*
    };
}

tags! { div span p h1 h2 h3 ul ol li button input textarea form label main header footer section nav em strong a pre code svg circle g table tr td th select option small }

impl<M: 'static> Element<M> {
    /// Set an attribute.
    pub fn attr(mut self, name: &'static str, value: impl ToString) -> Self {
        self.attrs.push((name, value.to_string()));
        self
    }
    /// Set `class`.
    pub fn class(self, value: impl ToString) -> Self {
        self.attr("class", value)
    }
    /// Set `id`.
    pub fn id(self, value: impl ToString) -> Self {
        self.attr("id", value)
    }
    /// Set `style`.
    pub fn style(self, value: impl ToString) -> Self {
        self.attr("style", value)
    }
    /// Set the diff key.
    pub fn key(mut self, key: impl ToString) -> Self {
        self.key = Some(key.to_string());
        self
    }
    /// Set the `value` property.
    pub fn value(mut self, value: impl ToString) -> Self {
        self.props.push(Prop::Value(value.to_string()));
        self
    }
    /// Set the `checked` property.
    pub fn checked(mut self, checked: bool) -> Self {
        self.props.push(Prop::Checked(checked));
        self
    }
    /// Add a listener.
    pub fn on(mut self, event: &'static str, f: impl Fn(&DomEvent) -> Option<M> + 'static) -> Self {
        self.listeners.push(Listener { event, handler: Rc::new(f), prevent_default: false });
        self
    }
    /// Add a listener that also prevents the default action.
    pub fn on_prevent(mut self, event: &'static str, f: impl Fn(&DomEvent) -> Option<M> + 'static) -> Self {
        self.listeners.push(Listener { event, handler: Rc::new(f), prevent_default: true });
        self
    }
    /// `click` → message.
    pub fn on_click(self, msg: M) -> Self
    where
        M: Clone,
    {
        self.on("click", move |_| Some(msg.clone()))
    }
    /// `input` → message from the value.
    pub fn on_input(self, f: impl Fn(String) -> M + 'static) -> Self {
        self.on("input", move |e| Some(f(e.value.clone())))
    }
    /// `change` → message from the value.
    pub fn on_change(self, f: impl Fn(&DomEvent) -> M + 'static) -> Self {
        self.on("change", move |e| Some(f(e)))
    }
    /// `submit` with default prevented → message.
    pub fn on_submit(self, f: impl Fn(&DomEvent) -> Option<M> + 'static) -> Self {
        self.on_prevent("submit", f)
    }
    /// Append a child.
    pub fn child(mut self, child: impl Into<Html<M>>) -> Self {
        self.children.push(child.into());
        self
    }
    /// Append a text child.
    pub fn text(self, s: impl Into<String>) -> Self {
        self.child(Html::Text(s.into()))
    }
    /// Append children.
    pub fn children(mut self, children: impl IntoIterator<Item = Html<M>>) -> Self {
        self.children.extend(children);
        self
    }
    /// Append a child only when `cond` holds.
    pub fn child_if(self, cond: bool, child: impl FnOnce() -> Html<M>) -> Self {
        if cond {
            self.child(child())
        } else {
            self
        }
    }
    /// Finish the element.
    pub fn build(self) -> Html<M> {
        Html::Element(self)
    }
}

impl<M> From<Element<M>> for Html<M> {
    fn from(e: Element<M>) -> Self {
        Html::Element(e)
    }
}

impl<M> From<&str> for Html<M> {
    fn from(s: &str) -> Self {
        Html::Text(s.to_string())
    }
}

impl<M> From<String> for Html<M> {
    fn from(s: String) -> Self {
        Html::Text(s)
    }
}

impl<M> Html<M> {
    /// Render as an HTML string. Listeners and properties are omitted. For tests and debugging.
    pub fn to_html_string(&self) -> String {
        let mut out = String::new();
        self.write_html(&mut out);
        out
    }

    fn write_html(&self, out: &mut String) {
        match self {
            Html::Text(t) => out.push_str(&escape(t)),
            Html::Element(e) => {
                out.push('<');
                out.push_str(e.tag);
                for (k, v) in &e.attrs {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    out.push_str(&escape(v));
                    out.push('"');
                }
                out.push('>');
                for c in &e.children {
                    c.write_html(out);
                }
                out.push_str("</");
                out.push_str(e.tag);
                out.push('>');
            }
        }
    }

    /// Find the first element with this tag, depth first.
    pub fn find(&self, tag: &str) -> Option<&Element<M>> {
        match self {
            Html::Text(_) => None,
            Html::Element(e) if e.tag == tag => Some(e),
            Html::Element(e) => e.children.iter().find_map(|c| c.find(tag)),
        }
    }
}

impl<M> Element<M> {
    /// Simulate an event on this element: run its listeners and collect messages. For tests.
    pub fn fire(&self, event: &str, dom_event: &DomEvent) -> Vec<M> {
        self.listeners.iter().filter(|l| l.event == event).filter_map(|l| (l.handler)(dom_event)).collect()
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
