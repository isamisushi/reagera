//! The classic todo app on Reagera. Every action is a minbin-encoded record in
//! a ConTime history; the session lives in the URL hash, so the URL itself is
//! the bug report.

use reagera::{html::*, minbin, App, Frame, Html};

/// 1. The wire format of every event. This is the whole contract of the app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Msg {
    #[default]
    ClearDone,
    Add { id: u32, title: String },
    Toggle(u32),
    Remove(u32),
    Filter(u8),
}

minbin::minbin_enum! { Msg [
    [0 => Self::ClearDone],
    [1 => Self::Add { id: u32, title: String }],
    [2 => Self::Toggle(id: u32)],
    [3 => Self::Remove(id: u32)],
    [4 => Self::Filter(filter: u8)]
] }

/// 2. The state and how each event changes it. Deterministic, no panics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Todo {
    pub id: u32,
    pub title: String,
    pub done: bool,
    pub created_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub todos: Vec<Todo>,
    pub filter: u8,
    pub next_id: u32,
}

pub struct TodoApp;

impl App for TodoApp {
    const NAME: &'static str = "todo/1";
    type Event = Msg;
    type State = State;

    fn apply(state: &mut State, event: &Msg, at_ms: i64) {
        match event {
            Msg::Add { id, title } => {
                state.next_id = state.next_id.max(id + 1);
                state.todos.push(Todo { id: *id, title: title.clone(), done: false, created_ms: at_ms });
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
    /// element; the element is keyed by `next_id`, so it is recreated empty after
    /// every add.
    fn view(state: &State, frame: &Frame) -> Html<Msg> {
        let visible = state.todos.iter().filter(|t| match state.filter {
            1 => !t.done,
            2 => t.done,
            _ => true,
        });
        let left = state.todos.iter().filter(|t| !t.done).count();
        let next_id = state.next_id.max(1);
        main()
            .class("app")
            .child(h1().text("todos"))
            .child(
                input()
                    .key(next_id)
                    .attr("placeholder", "What needs to be done?")
                    .attr("autofocus", "")
                    .attr("autocomplete", "off")
                    .on("keydown", move |e| {
                        let title = e.value.trim().to_string();
                        (e.key == "Enter" && !title.is_empty()).then_some(Msg::Add { id: next_id, title })
                    }),
            )
            .child(ul().class("list").children(visible.map(|t| {
                let id = t.id;
                li().key(id)
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

    #[test]
    fn add_toggle_clear() {
        let mut s: Store<TodoApp> = Store::new(Options::default().origin("t").clock(|| 1000));
        s.dispatch(Msg::Add { id: 1, title: "a".into() });
        s.dispatch(Msg::Add { id: 2, title: "b".into() });
        s.dispatch(Msg::Toggle(1));
        s.dispatch(Msg::ClearDone);
        assert_eq!(s.state().todos.iter().map(|t| t.id).collect::<Vec<_>>(), vec![2]);
        assert_eq!(s.state().next_id, 3);
        let html = TodoApp::view(s.state(), &s.frame());
        let msgs = html.find("input").unwrap().fire("keydown", &DomEvent { key: "Enter".into(), value: " c ".into(), ..DomEvent::default() });
        assert_eq!(msgs, vec![Msg::Add { id: 3, title: "c".into() }]);
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
        s.dispatch(Msg::Add { id: 1, title: "write the framework".into() });
        s.dispatch(Msg::Add { id: 2, title: "ship it".into() });
        s.dispatch(Msg::Toggle(1));
        s.backtrack(1_700_000_002_000);
        println!("SESSION={}", s.export_text());
    }
}
