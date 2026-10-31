//! Race: a search box where an early, slow answer arrives after a late, fast
//! one. This crate is the Reagera half. The React half is plain JavaScript in
//! `www/race/index.html`. The page sends each keystroke to both halves.
//!
//! The check in `resolve` is code that the app writes. Without it, Reagera has
//! the same bug as React. The difference is what stays after the race: every
//! answer is a record, so the race can be scrubbed, reloaded, and put in a test.

use reagera::{html::*, minbin, App, Ctx, Done, Frame, Html, Pending};

// ----- the wire format: events, requests, answers ----------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Msg {
    #[default]
    Clear,
    Type { text: String },
}

minbin::minbin_enum! { Msg [
    [0 => Self::Clear],
    [1 => Self::Type { text: String }]
] }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Effect {
    #[default]
    Nothing,
    Search { query: String },
}

minbin::minbin_enum! { Effect [
    [0 => Self::Nothing],
    [1 => Self::Search { query: String }]
] }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    #[default]
    Nothing,
    Found { names: Vec<String> },
}

minbin::minbin_enum! { Outcome [
    [0 => Self::Nothing],
    [1 => Self::Found { names: Vec<String> }]
] }

// ----- the state and the pure rules --------------------------------------------

/// One answer, in arrival order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    pub query: String,
    pub count: u32,
    pub used: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub query: String,
    pub results: Vec<String>,
    /// The query that `results` answers.
    pub results_for: String,
    pub answers: Vec<Answer>,
}

pub struct Search;

impl App for Search {
    const NAME: &'static str = "race/1";
    type Event = Msg;
    type State = State;
    type Effect = Effect;
    type Outcome = Outcome;

    fn apply(state: &mut State, event: &Msg, ctx: &mut Ctx<Self>) {
        match event {
            Msg::Type { text } => {
                state.query = text.clone();
                if text.is_empty() {
                    state.results.clear();
                    state.results_for.clear();
                } else {
                    ctx.emit(Effect::Search { query: text.clone() });
                }
            }
            Msg::Clear => *state = State::default(),
        }
    }

    /// `resolve` gets the request and the state of this moment, so it can
    /// compare the query of the answer with the query in the box.
    fn resolve(state: &mut State, request: &Effect, outcome: &Outcome, _ctx: &mut Ctx<Self>) {
        let (Effect::Search { query }, Outcome::Found { names }) = (request, outcome) else { return };
        let used = *query == state.query;
        state.answers.push(Answer { query: query.clone(), count: names.len() as u32, used });
        if used {
            state.results = names.clone();
            state.results_for = query.clone();
        }
    }

    /// The only place that touches the outside world. Never called on replay.
    fn perform(pending: &Pending<Effect>, done: Done<Outcome>) {
        #[cfg(target_arch = "wasm32")]
        if let Effect::Search { query } = &pending.request {
            let names = search(query);
            reagera::web::after(delay_ms(query), move || done(Outcome::Found { names }));
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (pending, done);
    }

    fn view(state: &State, frame: &Frame) -> Html<Msg> {
        div()
            .class("app")
            .child(input().attr("readonly", "").attr("placeholder", "the query").value(&state.query))
            .child(p().class("status").text(format!("{} in flight, list shows {:?}", frame.pending, state.results_for)))
            .child(ul().class("list").children(state.results.iter().map(|name| li().key(name).text(name).build())))
            .child(table().class("answers").child(tr().child(th().text("answer for")).child(th().text("names")).child(th().text("result"))).children(state.answers.iter().enumerate().map(|(i, a)| {
                tr().key(i)
                    .class(if a.used { "used" } else { "ignored" })
                    .child(td().text(format!("{:?}", a.query)))
                    .child(td().text(a.count.to_string()))
                    .child(td().text(if a.used { "used" } else { "stale, ignored" }))
                    .build()
            })))
            .child_if(!frame.live, || em().class("past").text("viewing the past").build())
            .build()
    }
}

// ----- the outside world: a fake server, the same as the one on the page --------

const CATALOG: &[&str] = &["elm", "go", "preact", "python", "react", "reagera", "redux", "ruby", "rust", "svelte", "vue"];

/// The names that start with the query.
pub fn search(query: &str) -> Vec<String> {
    CATALOG.iter().filter(|name| name.starts_with(query)).map(|name| name.to_string()).collect()
}

/// A short query matches more names, so its answer takes longer.
pub fn delay_ms(query: &str) -> i32 {
    match query.chars().count() {
        0 | 1 => 1800,
        2 => 1200,
        3 => 600,
        _ => 300,
    }
}

// ----- the page's entry points -----------------------------------------------

#[cfg(target_arch = "wasm32")]
mod page {
    use super::*;
    use reagera::web::{mount, MountOptions, Runtime};
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;

    thread_local! {
        static RUNTIME: RefCell<Option<Runtime<Search>>> = const { RefCell::new(None) };
    }

    fn send(msg: Msg) {
        RUNTIME.with(|r| {
            if let Some(rt) = r.borrow().as_ref() {
                rt.dispatch(msg);
            }
        });
    }

    /// The page's search box calls this for each keystroke.
    #[wasm_bindgen]
    pub fn type_query(text: &str) {
        send(Msg::Type { text: text.to_string() });
    }

    #[wasm_bindgen]
    pub fn clear() {
        send(Msg::Clear);
    }

    #[wasm_bindgen(start)]
    fn start() {
        let rt = mount::<Search>("reagera", MountOptions { devtools: true, session_in_url: true, origin: None });
        RUNTIME.with(|r| *r.borrow_mut() = Some(rt));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reagera::{Body, Options, Store};

    fn store() -> Store<Search> {
        let mut t = 1000;
        Store::new(Options::default().origin("tab").clock(move || {
            t += 10;
            t
        }))
    }

    fn type_keys(s: &mut Store<Search>, keys: &[&str]) {
        for text in keys {
            s.dispatch(Msg::Type { text: text.to_string() }).unwrap();
        }
    }

    /// Answer a pending search. The test picks the order, so it needs no timers.
    fn answer(s: &mut Store<Search>, query: &str) {
        let id = s.pending().iter().find(|p| p.request == Effect::Search { query: query.to_string() }).expect("pending").id;
        s.reply(id, Outcome::Found { names: search(query) }).unwrap();
    }

    #[test]
    fn an_answer_that_arrives_after_a_newer_one_is_ignored() {
        let mut s = store();
        type_keys(&mut s, &["r", "re", "rea"]);
        assert_eq!(s.pending().len(), 3);

        answer(&mut s, "rea");
        answer(&mut s, "re");
        answer(&mut s, "r");

        assert_eq!(s.state().results, vec!["react", "reagera"]);
        assert_eq!(s.state().results_for, "rea");
        assert_eq!(s.state().answers.iter().map(|a| a.used).collect::<Vec<_>>(), vec![true, false, false]);
    }

    #[test]
    fn the_session_keeps_the_order_of_the_race() {
        let mut s = store();
        type_keys(&mut s, &["r", "re", "rea"]);
        answer(&mut s, "rea");
        answer(&mut s, "re");
        answer(&mut s, "r");

        let mut copy = Store::<Search>::import_text(&s.export_text(), Options::default()).unwrap();
        assert_eq!(copy.state(), s.state());
        assert!(copy.pending().is_empty());
        let replies = copy.events().iter().filter(|r| matches!(r.body(), Body::Reply { .. })).count();
        assert_eq!(replies, 3);
    }

    #[test]
    fn the_view_marks_an_ignored_answer() {
        let mut s = store();
        type_keys(&mut s, &["r", "re"]);
        answer(&mut s, "re");
        answer(&mut s, "r");
        let html = Search::view(s.state(), &s.frame()).to_html_string();
        assert!(html.contains("stale, ignored"));
    }

    #[test]
    fn a_shorter_query_takes_longer() {
        assert!(delay_ms("r") > delay_ms("re"));
        assert!(delay_ms("re") > delay_ms("rea"));
        assert_eq!(search("rea"), vec!["react", "reagera"]);
    }
}
