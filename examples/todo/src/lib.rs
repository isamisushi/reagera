//! The classic todo app on Reagera. Every action is a minbin-encoded record in
//! a ConTime history; the session lives in the URL hash, so the URL itself is
//! the bug report.

use reagera::{html::*, minbin, App, Frame, Html, Record};

/// 1. The wire format of every event. This is the whole contract of the app.
/// A todo's id is the content ID of the record that added it, assigned in
/// `apply`. The view never decides an id, so two adds before a re-render
/// cannot collide.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Msg {
    #[default]
    ClearDone,
    Add { title: String },
    Toggle(u128),
    Remove(u128),
    Filter(u8),
}

minbin::minbin_enum! { Msg [
    [0 => Self::ClearDone],
    [1 => Self::Add { title: String }],
    [2 => Self::Toggle(id: u128)],
    [3 => Self::Remove(id: u128)],
    [4 => Self::Filter(filter: u8)]
] }

/// 2. The state and how each event changes it. Deterministic, no panics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Todo {
    pub id: u128,
    pub title: String,
    pub done: bool,
    pub created_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub todos: Vec<Todo>,
    pub filter: u8,
    pub adds: u32,
}

pub struct TodoApp;

impl App for TodoApp {
    const NAME: &'static str = "todo/1";
    type Event = Msg;
    type State = State;

    fn apply(state: &mut State, record: &Record<Msg>) {
        match record.body() {
            Msg::Add { title } => {
                state.adds += 1;
                state.todos.push(Todo { id: record.id(), title: title.clone(), done: false, created_ms: record.ms() });
            }
            Msg::Toggle(id) => {
                if let Some(t) = state.todos.iter_mut().find(|t| t.id == *id) {
                    t.done = !t.done;
                }
            }
            Msg::Remove(id) => state.todos.retain(|t| t.id != *id),
            Msg::ClearDone => state.todos.retain(|t| !t.done),
            Msg::Filter(f) => state.filter = *f,
        }
    }

    /// 3. The view: a pure function of state. The draft text stays in the input
    /// element; the element is keyed by the number of adds, so it is recreated
    /// empty after every add.
    fn view(state: &State, frame: &Frame) -> Html<Msg> {
        let visible = state.todos.iter().filter(|t| match state.filter {
            1 => !t.done,
            2 => t.done,
            _ => true,
        });
        let left = state.todos.iter().filter(|t| !t.done).count();
        let adds = state.adds;
        main()
            .class("app")
            .child(h1().text("todos"))
            .child(
                input()
                    .key(adds)
                    .attr("placeholder", "What needs to be done?")
                    .attr("autofocus", "")
                    .attr("autocomplete", "off")
                    .on("keydown", move |e| {
                        let title = e.value.trim().to_string();
                        (e.key == "Enter" && !title.is_empty()).then_some(Msg::Add { title })
                    }),
            )
            .child(ul().class("list").children(visible.map(|t| {
                let id = t.id;
                li().key(format!("{id:x}"))
                    .class(if t.done { "done" } else { "" })
                    .child(input().attr("type", "checkbox").checked(t.done).on("change", move |_| Some(Msg::Toggle(id))))
                    .child(span().text(&t.title))
                    .child(button().class("x").on_click(Msg::Remove(id)).text("×"))
                    .build()
            })))
            .child(
                footer()
                    .child(span().text(format!("{left} left")))
                    .children([("all", 0u8), ("active", 1), ("done", 2)].into_iter().map(|(label, f)| {
                        button().class(if state.filter == f { "on" } else { "" }).on_click(Msg::Filter(f)).text(label).build()
                    }))
                    .child(button().on_click(Msg::ClearDone).text("clear done"))
                    .child_if(!frame.live, || em().text(" viewing the past ").build()),
            )
            .build()
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
fn start() {
    use reagera::web::{mount, MountOptions};
    let rt = mount::<TodoApp>("root", MountOptions { devtools: true, session_in_url: true, origin: None });
    std::mem::forget(rt);
}

#[cfg(test)]
mod tests {
    use super::*;
    use reagera::{Options, Store};

    /// Two adds from the same rendered tree, before any re-render, make two todos.
    #[test]
    fn stale_handlers_cannot_collide() {
        let mut s: Store<TodoApp> = Store::new(Options::default().origin("t").clock(|| 1000));
        let html = TodoApp::view(s.state(), &s.frame());
        let input = html.find("input").unwrap();
        for title in ["x", "y"] {
            for m in input.fire("keydown", &DomEvent { key: "Enter".into(), value: title.into(), ..DomEvent::default() }) {
                s.dispatch(m).unwrap();
            }
        }
        let ids: Vec<_> = s.state().todos.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn add_toggle_clear() {
        let mut s: Store<TodoApp> = Store::new(Options::default().origin("t").clock(|| 1000));
        let a = s.dispatch(Msg::Add { title: "a".into() }).unwrap();
        let b = s.dispatch(Msg::Add { title: "b".into() }).unwrap();
        s.dispatch(Msg::Toggle(a.id())).unwrap();
        s.dispatch(Msg::ClearDone).unwrap();
        assert_eq!(s.state().todos.iter().map(|t| t.id).collect::<Vec<_>>(), vec![b.id()]);
        assert_eq!(s.state().adds, 2);
        let html = TodoApp::view(s.state(), &s.frame());
        let msgs = html.find("input").unwrap().fire("keydown", &DomEvent { key: "Enter".into(), value: " c ".into(), ..DomEvent::default() });
        assert_eq!(msgs, vec![Msg::Add { title: "c".into() }]);
        let copy: Store<TodoApp> = Store::import(&s.export(), Options::default()).unwrap();
        assert_eq!(copy.state(), s.state());
    }
}

#[cfg(test)]
mod seed {
    use super::*;
    use reagera::{Options, Store};

    /// `cargo test -p reagera-example-todo seed -- --ignored --nocapture` prints a session for manual checks.
    #[test]
    #[ignore]
    fn print_session() {
        let mut t = 1_700_000_000_000;
        let mut s: Store<TodoApp> = Store::new(Options::default().origin("seed").clock(move || {
            t += 1000;
            t
        }));
        let first = s.dispatch(Msg::Add { title: "write the framework".into() }).unwrap();
        s.dispatch(Msg::Add { title: "ship it".into() }).unwrap();
        s.dispatch(Msg::Toggle(first.id())).unwrap();
        s.backtrack(1_700_000_002_000);
        println!("SESSION={}", s.export_text());
    }
}
