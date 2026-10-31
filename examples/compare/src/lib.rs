//! Compare: the same todo app in React and in Reagera, side by side, with what
//! each framework keeps shown under it. This crate is the Reagera half. The
//! React half is plain JavaScript in `www/compare/index.html`. The page sends
//! the same action to both halves through [`script`].

use reagera::{html::*, minbin, App, Ctx, Frame, Html, Nothing};

// ----- the app: the same rules as the React reducer on the page ---------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Msg {
    #[default]
    ClearDone,
    Add { title: String },
    Toggle(u128),
    Remove(u128),
}

minbin::minbin_enum! { Msg [
    [0 => Self::ClearDone],
    [1 => Self::Add { title: String }],
    [2 => Self::Toggle(id: u128)],
    [3 => Self::Remove(id: u128)]
] }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Todo {
    pub id: u128,
    pub title: String,
    pub done: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub todos: Vec<Todo>,
    pub adds: u32,
}

pub struct Todos;

impl App for Todos {
    const NAME: &'static str = "compare/1";
    type Event = Msg;
    type State = State;
    type Effect = Nothing;
    type Outcome = Nothing;

    /// The React reducer on the page does the same thing to the same state.
    /// The one difference: a todo's id is the content ID of the record that
    /// added it, so a replay gives the same ids.
    fn apply(state: &mut State, event: &Msg, ctx: &mut Ctx<Self>) {
        match event {
            Msg::Add { title } => {
                state.adds += 1;
                state.todos.push(Todo { id: ctx.id(), title: title.clone(), done: false });
            }
            Msg::Toggle(id) => {
                if let Some(t) = state.todos.iter_mut().find(|t| t.id == *id) {
                    t.done = !t.done;
                }
            }
            Msg::Remove(id) => state.todos.retain(|t| t.id != *id),
            Msg::ClearDone => state.todos.retain(|t| !t.done),
        }
    }

    fn view(state: &State, frame: &Frame) -> Html<Msg> {
        let left = state.todos.iter().filter(|t| !t.done).count();
        let mut field = input()
            .key(state.adds)
            .attr("placeholder", if frame.live { "What needs to be done?" } else { "viewing the past: read-only" })
            .attr("autocomplete", "off")
            .on("keydown", move |e| {
                let title = e.value.trim().to_string();
                (e.key == "Enter" && !title.is_empty()).then_some(Msg::Add { title })
            });
        if !frame.live {
            field = field.attr("disabled", "");
        }
        div()
            .class("app")
            .child(field)
            .child(ul().class("list").children(state.todos.iter().map(|t| {
                let id = t.id;
                li().key(format!("{id:x}"))
                    .class(if t.done { "done" } else { "" })
                    .child(input().attr("type", "checkbox").checked(t.done).on("change", move |_| Some(Msg::Toggle(id))))
                    .child(span().text(&t.title))
                    .child(button().class("x").on_click(Msg::Remove(id)).text("×"))
                    .build()
            })))
            .child(footer().child(span().text(format!("{left} left"))).child(button().on_click(Msg::ClearDone).text("clear done")))
            .build()
    }
}

/// An event as text, with short ids.
pub fn event_text(event: &Msg) -> String {
    match event {
        Msg::Add { title } => format!("Add {{ title: {title:?} }}"),
        Msg::Toggle(id) => format!("Toggle({})", reagera::effect::short(*id)),
        Msg::Remove(id) => format!("Remove({})", reagera::effect::short(*id)),
        Msg::ClearDone => "ClearDone".to_string(),
    }
}

/// The state as text, for the panel. The page prints the React state the same way.
pub fn state_text(state: &State) -> String {
    let mut s = String::from("{\n  todos: [\n");
    for t in &state.todos {
        s.push_str(&format!("    {{ id: {}, title: {:?}, done: {} }},\n", reagera::effect::short(t.id), t.title, t.done));
    }
    s.push_str(&format!("  ],\n  adds: {},\n}}", state.adds));
    s
}

// ----- the page: the app, and a panel that shows what the store keeps -----------

#[cfg(target_arch = "wasm32")]
mod page {
    use super::*;
    use reagera::effect::short;
    use reagera::web::{every, next_frame, Renderer};
    use reagera::{Options, Store};
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::prelude::*;

    /// Messages of the page. Only `App` goes into the log.
    #[derive(Clone, Debug)]
    pub enum PageMsg {
        App(Msg),
        Backtrack(i64),
        StepBack,
        StepForward,
        Live,
        Replay,
        Export,
    }

    pub struct Page {
        pub store: Store<Todos>,
        exported: Option<String>,
        pub replaying: bool,
    }

    impl Page {
        pub fn handle(&mut self, msg: PageMsg) {
            match msg {
                PageMsg::App(m) => {
                    if let Err(e) = self.store.dispatch(m) {
                        reagera::web::log(&format!("reagera: {e}"));
                    }
                }
                PageMsg::Backtrack(ms) => {
                    self.replaying = false;
                    self.store.backtrack(ms);
                }
                PageMsg::StepBack => {
                    self.replaying = false;
                    self.step_back();
                }
                PageMsg::StepForward => self.step_forward(),
                PageMsg::Live => {
                    self.replaying = false;
                    self.store.go_live();
                }
                PageMsg::Replay => {
                    if !self.store.is_empty() {
                        self.store.backtrack(self.store.earliest_ms() - 1);
                        self.replaying = true;
                    }
                }
                PageMsg::Export => self.exported = Some(self.store.export_text()),
            }
        }

        fn step_back(&mut self) {
            let head = self.store.head_ms();
            let before = self.store.events_between(i64::MIN, head - 1).last().map(|r| r.ms());
            let earliest = self.store.earliest_ms();
            self.store.backtrack(before.unwrap_or(earliest - 1));
        }

        /// One event later. Returns to live after the newest one.
        pub fn step_forward(&mut self) {
            let head = self.store.head_ms();
            match self.store.events_between(head, i64::MAX).first().map(|r| r.ms()) {
                Some(ms) => self.store.backtrack(ms),
                None => {
                    self.store.go_live();
                    self.replaying = false;
                }
            }
        }

        /// Seconds after the first record.
        fn rel(&self, ms: i64) -> String {
            format!("+{:.1}s", (ms - self.store.earliest_ms()) as f64 / 1000.0)
        }

        pub fn view(&mut self) -> Html<PageMsg> {
            let app = Todos::view(self.store.state(), &self.store.frame()).map(PageMsg::App);
            let events = self.store.events();
            let live = self.store.live();
            let head = self.store.head_ms();
            let earliest = self.store.earliest_ms();
            let latest = self.store.latest_ms();
            let applied = events.iter().filter(|r| r.ms() <= head).count();
            let head_id = events.iter().filter(|r| r.ms() <= head).last().map(|r| r.id());
            let state = state_text(self.store.state());
            let exported = self.exported.clone();
            let replaying = self.replaying;
            let position = if events.is_empty() {
                "no events yet".to_string()
            } else if live {
                format!("live: all {} events applied", events.len())
            } else if applied == 0 {
                "before the first event".to_string()
            } else {
                format!("{} of {} events applied, head at {}", applied, events.len(), self.rel(head))
            };

            div()
                .child(app)
                .child(
                    div()
                        .class("inside")
                        .child(h3().text("what Reagera keeps"))
                        .child(
                            section()
                                .child(h4().text(format!("1. the log: {} records", events.len())))
                                .child(p().class("note").text("Every action is a record with a time, an origin, and a content ID. Click a row to show the state as of that record."))
                                .child(table().class("log").child(tr().child(th().text("#")).child(th().text("time")).child(th().text("event")).child(th().text("id"))).children(
                                    events.iter().enumerate().map(|(i, r)| {
                                        let ms = r.ms();
                                        let class = if Some(r.id()) == head_id { "head" } else if ms <= head { "past" } else { "future" };
                                        tr().key(format!("{:x}", r.id()))
                                            .class(class)
                                            .on("click", move |_| Some(PageMsg::Backtrack(ms)))
                                            .child(td().text((i + 1).to_string()))
                                            .child(td().text(self.rel(ms)))
                                            .child(td().text(match r.body() {
                                                reagera::Body::Event(e) => event_text(e),
                                                other => format!("{other:?}"),
                                            }))
                                            .child(td().text(short(r.id())))
                                            .build()
                                    }),
                                )),
                        )
                        .child(
                            section()
                                .child(h4().text("2. the fold: apply × ".to_string() + &applied.to_string() + " → state"))
                                .child(p().class("note").text("The state is not stored. It is computed by applying the records at or before the head, in time order, to the default state."))
                                .child(pre().class("state").text(state)),
                        )
                        .child(
                            section()
                                .child(h4().text("3. time travel: move the head"))
                                .child(p().class("note").text("Moving the head changes which records the fold reads. Nothing is deleted, and the app above is read-only until you press live."))
                                .child(div().class("position").class(if live { "position live" } else { "position past" }).text(position))
                                .child(
                                    input()
                                        .attr("type", "range")
                                        .attr("min", earliest - 1)
                                        .attr("max", latest.max(earliest))
                                        .attr("step", 1)
                                        .value(head)
                                        .on("input", |e| e.value.parse().ok().map(PageMsg::Backtrack)),
                                )
                                .child(
                                    div()
                                        .class("row")
                                        .child(btn("⟵ step back", PageMsg::StepBack))
                                        .child(btn("step forward ⟶", PageMsg::StepForward))
                                        .child(btn(if replaying { "replaying…" } else { "replay from the start" }, PageMsg::Replay))
                                        .child(btn("live", PageMsg::Live)),
                                ),
                        )
                        .child(
                            section()
                                .child(h4().text("4. the session: the log as bytes"))
                                .child(p().class("note").text("The export is the log, encoded with minbin. Reload this page: Reagera rebuilds from the URL hash. Paste the text into a test to reproduce a bug."))
                                .child(div().class("row").child(btn("export", PageMsg::Export)))
                                .child_if(exported.is_some(), || {
                                    let text = exported.clone().unwrap_or_default();
                                    div()
                                        .child(p().class("note").text(format!("{} characters of base64 for {} records", text.len(), events.len())))
                                        .child(textarea().attr("readonly", "").attr("rows", 3).value(text.clone()))
                                        .child(a().attr("href", format!("/compare/#{text}")).attr("target", "_blank").text("open this session in a new tab"))
                                        .build()
                                }),
                        ),
                )
                .build()
        }
    }

    fn btn(label: &str, msg: PageMsg) -> Html<PageMsg> {
        button().on_click(msg).text(label).build()
    }

    thread_local! {
        static PAGE: RefCell<Option<(Rc<RefCell<Page>>, Rc<dyn Fn()>)>> = const { RefCell::new(None) };
    }

    /// The page's script bar sends the same action to both halves. This is the
    /// Reagera half's entry: `add`, `toggle-first`, `remove-first`, `clear-done`.
    #[wasm_bindgen]
    pub fn script(action: &str, arg: &str) {
        PAGE.with(|p| {
            let Some((page, render)) = p.borrow().clone() else { return };
            let msg = {
                let page = page.borrow();
                let first = page.store.state().todos.first().map(|t| t.id);
                match action {
                    "add" => Some(Msg::Add { title: arg.to_string() }),
                    "toggle-first" => first.map(Msg::Toggle),
                    "remove-first" => first.map(Msg::Remove),
                    "clear-done" => Some(Msg::ClearDone),
                    _ => None,
                }
            };
            if let Some(m) = msg {
                page.borrow_mut().handle(PageMsg::App(m));
                render();
            }
        });
    }

    #[wasm_bindgen(start)]
    fn start() {
        let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
        let store = match hash.strip_prefix('#').filter(|h| !h.is_empty()) {
            Some(text) => Store::<Todos>::import_text(text, Options::default()).unwrap_or_else(|_| Store::new(Options::default())),
            None => Store::new(Options::default()),
        };
        let page = Rc::new(RefCell::new(Page { store, exported: None, replaying: false }));
        let root = reagera::web::document().get_element_by_id("reagera").expect("#reagera");
        let renderer: Rc<RefCell<Option<Renderer<PageMsg>>>> = Rc::new(RefCell::new(None));

        let render: Rc<dyn Fn()> = {
            let page = page.clone();
            let renderer = renderer.clone();
            let scheduled = Rc::new(std::cell::Cell::new(false));
            Rc::new(move || {
                if scheduled.replace(true) {
                    return;
                }
                let page = page.clone();
                let renderer = renderer.clone();
                let scheduled = scheduled.clone();
                next_frame(move || {
                    scheduled.set(false);
                    let (tree, hash) = {
                        let mut p = page.borrow_mut();
                        let tree = p.view();
                        let hash = p.store.export_text();
                        (tree, hash)
                    };
                    if let Some(r) = renderer.borrow_mut().as_mut() {
                        r.render(tree);
                    }
                    if let Some(w) = web_sys::window() {
                        let _ = w.history().and_then(|h| h.replace_state_with_url(&JsValue::NULL, "", Some(&format!("#{hash}"))));
                    }
                });
            })
        };

        let dispatch: Rc<dyn Fn(PageMsg)> = {
            let page = page.clone();
            let render = render.clone();
            Rc::new(move |msg| {
                page.borrow_mut().handle(msg);
                render();
            })
        };
        let mut first = Renderer::new(root, dispatch);
        first.render(page.borrow_mut().view());
        *renderer.borrow_mut() = Some(first);

        {
            let page = page.clone();
            let render = render.clone();
            every(600, move || {
                let mut p = page.borrow_mut();
                if p.replaying {
                    p.step_forward();
                    drop(p);
                    render();
                }
            });
        }
        PAGE.with(|p| *p.borrow_mut() = Some((page, render)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reagera::{Options, Store};

    #[test]
    fn the_fold_gives_the_state_at_any_event() {
        let mut t = 1000;
        let mut s: Store<Todos> = Store::new(Options::default().origin("t").clock(move || {
            t += 1000;
            t
        }));
        let milk = s.dispatch(Msg::Add { title: "milk".into() }).unwrap(); // 2000
        s.dispatch(Msg::Add { title: "eggs".into() }).unwrap(); // 3000
        s.dispatch(Msg::Toggle(milk.id())).unwrap(); // 4000
        s.dispatch(Msg::ClearDone).unwrap(); // 5000
        assert_eq!(s.state().todos.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), vec!["eggs"]);
        assert_eq!(s.state_at(3000).todos.len(), 2);
        assert!(s.state_at(4000).todos[0].done);
        assert!(s.state_at(1999).todos.is_empty());
        let copy = Store::<Todos>::import_text(&s.export_text(), Options::default()).unwrap();
        assert_eq!(copy.state(), s.state());
        assert!(state_text(s.state()).contains("title: \"eggs\""));
        assert_eq!(event_text(&Msg::Toggle(milk.id())), format!("Toggle({})", reagera::effect::short(milk.id())));
    }
}
