use reagera::{html::*, minbin, App, Body, Ctx, Frame, Html, Nothing, Options, Record, Store};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
enum Msg {
    #[default]
    Reset,
    Add(i64),
    Rename { label: String, note: Option<String> },
}

minbin::minbin_enum! { Msg [
    [0 => Self::Reset],
    [1 => Self::Add(n: i64)],
    [2 => Self::Rename { label: String, note: Option<String> }]
] }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    count: i64,
    label: String,
    applied: Vec<i64>,
}

struct Counter;

impl App for Counter {
    const NAME: &'static str = "counter";
    type Event = Msg;
    type State = State;
    type Effect = Nothing;
    type Outcome = Nothing;

    fn apply(state: &mut State, event: &Msg, ctx: &mut Ctx<Self>) {
        state.applied.push(ctx.ms());
        match event {
            Msg::Reset => state.count = 0,
            Msg::Add(n) => state.count += n,
            Msg::Rename { label, .. } => state.label = label.clone(),
        }
    }

    fn view(state: &State, frame: &Frame) -> Html<Msg> {
        div()
            .child(button().on_click(Msg::Add(1)).text("+"))
            .child(span().text(state.count.to_string()))
            .child_if(!frame.live, || em().text("past").build())
            .build()
    }
}

trait NextOrLast {
    fn get_mut_next(&self) -> i64;
}
impl NextOrLast for Cell<std::vec::IntoIter<i64>> {
    fn get_mut_next(&self) -> i64 {
        let mut it = self.replace(Vec::new().into_iter());
        let v = it.next().unwrap_or(0);
        self.set(it);
        v
    }
}

fn ticking(start: i64, step: i64) -> impl FnMut() -> i64 {
    let t = Rc::new(Cell::new(start - step));
    move || {
        t.set(t.get() + step);
        t.get()
    }
}

fn store(origin: &str, start: i64, step: i64) -> Store<Counter> {
    Store::new(Options::default().origin(origin).clock(ticking(start, step)))
}

#[test]
fn state_is_a_fold_over_the_log() {
    let mut s = store("a", 1000, 10);
    s.dispatch(Msg::Add(2)).unwrap();
    s.dispatch(Msg::Add(3)).unwrap();
    assert_eq!(s.state().count, 5);
    assert_eq!(s.events().len(), 2);
    assert_eq!(s.head_ms(), 1010);
    assert!(s.live());
    assert!(s.take_rejections().is_empty());
}

#[test]
fn state_at_answers_any_time_without_moving_the_head() {
    let mut s = store("a", 1000, 10);
    for i in 1..=10 {
        s.dispatch(Msg::Add(i)).unwrap();
    }
    assert_eq!(s.state_at(999).count, 0);
    assert_eq!(s.state_at(1000).count, 1);
    assert_eq!(s.state_at(1045).count, 15);
    assert_eq!(s.state_at(5000).count, 55);
    assert_eq!(s.state().count, 55);
    assert!(s.live());
}

#[test]
fn checkpoints_agree_with_full_replay() {
    let mut a: Store<Counter> = Store::new(Options { checkpoint_interval: 3, ..Options::default() }.origin("a").clock(ticking(1000, 10)));
    let mut b: Store<Counter> = Store::new(Options { checkpoint_interval: 1_000_000, ..Options::default() }.origin("a").clock(ticking(1000, 10)));
    for i in 1..=50 {
        let m = if i % 7 == 0 { Msg::Reset } else { Msg::Add(i) };
        a.dispatch(m.clone()).unwrap();
        b.dispatch(m).unwrap();
    }
    for t in (990..1500).step_by(7) {
        assert_eq!(a.state_at(t), b.state_at(t), "t={t}");
    }
}

#[test]
fn backtrack_shows_the_past_and_loses_nothing() {
    let mut s = store("a", 1000, 10);
    for _ in 0..3 {
        s.dispatch(Msg::Add(1)).unwrap();
    }
    s.backtrack(1010);
    assert_eq!(s.state().count, 2);
    assert!(!s.live());
    assert_eq!(s.events().len(), 3);
    assert_eq!(s.frame(), Frame { live: false, head_ms: 1010, events: 3, pending: 0 });
    s.go_live();
    assert_eq!(s.state().count, 3);
}

#[test]
fn time_travel_is_read_only() {
    let mut s = store("a", 1000, 10);
    s.dispatch(Msg::Add(1)).unwrap();
    s.dispatch(Msg::Add(10)).unwrap();
    s.backtrack(1000);
    assert_eq!(s.dispatch(Msg::Add(5)), Err(reagera::StoreError::NotLive));
    assert_eq!(s.events().len(), 2, "nothing is dropped");
    s.go_live();
    s.dispatch(Msg::Add(5)).unwrap();
    assert_eq!(s.state().count, 16);
}

#[test]
fn dispatch_time_never_goes_backwards() {
    // A clock that jumps back, as NTP or a sleeping laptop can make Date.now() do.
    let times = Rc::new(Cell::new(vec![5000, 4000, 4500].into_iter()));
    let t = times.clone();
    let mut s: Store<Counter> = Store::new(Options::default().origin("a").clock(move || t.get_mut_next()));
    let r1 = s.dispatch(Msg::Add(1)).unwrap();
    let r2 = s.dispatch(Msg::Reset).unwrap();
    let r3 = s.dispatch(Msg::Add(2)).unwrap();
    assert_eq!((r1.ms(), r2.ms(), r3.ms()), (5000, 5000, 5000));
    assert_eq!(s.state().count, 2, "reset is applied after the first add, as made");
    let _ = times;
}

#[test]
fn a_reply_is_ordered_after_what_was_seen() {
    let mut a = store("a", 1000, 10); // a's clock is behind
    let mut b = store("b", 9000, 10);
    let from_b = b.dispatch(Msg::Add(1)).unwrap(); // 9000
    a.receive(from_b);
    let reply = a.dispatch(Msg::Reset).unwrap();
    assert_eq!(reply.ms(), 9000);
    assert_eq!(a.state().count, 0);
    assert_eq!(a.engine_status().histories, 1);
}

#[test]
fn a_reply_is_ordered_after_what_was_seen_within_the_same_millisecond() {
    // b has made several events, so its sequence numbers are ahead of a's.
    // a's reply lands in b's millisecond and must sort after b's events there.
    let mut a = store("a", 1000, 10);
    let mut b = store("b", 9000, 0); // every event of b is at 9000
    let add1 = b.dispatch(Msg::Add(1)).unwrap(); // 9000, seq 0
    let add10 = b.dispatch(Msg::Add(10)).unwrap(); // 9000, seq 1
    a.receive(add1);
    a.receive(add10);
    let reply = a.dispatch(Msg::Reset).unwrap();
    assert_eq!((reply.ms(), reply.seq()), (9000, 2));
    assert_eq!(a.state().count, 0, "reset applies after both adds it saw");
    b.receive(reply);
    assert_eq!(b.state().count, 0, "b agrees");
    let later = a.dispatch(Msg::Add(3)).unwrap();
    assert_eq!(later.seq(), 3, "a's own sequence stays monotonic after the jump");
}

#[test]
fn origins_are_random_by_default() {
    let one: Store<Counter> = Store::new(Options::default().clock(|| 42));
    let two: Store<Counter> = Store::new(Options::default().clock(|| 42));
    assert_eq!(one.origin().len(), 8);
    assert_ne!(one.origin(), two.origin(), "same clock, different origins");
}

#[test]
fn late_events_land_by_their_own_time() {
    let mut a = store("a", 1000, 100);
    let mut b = store("b", 1050, 100);
    let a1 = a.dispatch(Msg::Add(1)).unwrap(); // 1000
    let a2 = a.dispatch(Msg::Reset).unwrap(); // 1100
    let b1 = b.dispatch(Msg::Add(7)).unwrap(); // 1050, arrives late at a
    assert!(a.receive(b1.clone()));
    let ids: Vec<_> = a.events().iter().map(Record::id).collect();
    assert_eq!(ids, vec![a1.id(), b1.id(), a2.id()]);
    assert_eq!(a.state_at(1050).count, 8);
    assert_eq!(a.state().count, 0);
    assert_eq!(a.state().applied, vec![1000, 1050, 1100]);

    // Arrival order does not matter.
    let mut c = store("c", 0, 1);
    c.receive(a2.clone());
    c.receive(b1.clone());
    c.receive(a1.clone());
    let ids: Vec<_> = c.events().iter().map(Record::id).collect();
    assert_eq!(ids, vec![a1.id(), b1.id(), a2.id()]);
    assert_eq!(c.state(), a.state());
}

#[test]
fn duplicates_are_ignored() {
    let mut a = store("a", 1000, 10);
    let r = a.dispatch(Msg::Add(1)).unwrap();
    assert!(!a.receive(r));
    assert_eq!(a.state().count, 1);
    assert_eq!(a.events().len(), 1);
}

#[test]
fn same_millisecond_keeps_dispatch_order() {
    let mut s: Store<Counter> = Store::new(Options::default().origin("a").clock(|| 1000));
    s.dispatch(Msg::Add(1)).unwrap();
    s.dispatch(Msg::Reset).unwrap();
    s.dispatch(Msg::Add(2)).unwrap();
    assert_eq!(s.state().count, 2);
    assert_eq!(s.state_at(1000).count, 2);
}

#[test]
fn export_and_import_reproduce_the_identical_history() {
    let mut s = store("tab1", 1_700_000_000_000, 37);
    for i in 1..=20 {
        s.dispatch(Msg::Add(i)).unwrap();
    }
    s.dispatch(Msg::Rename { label: "héllo".into(), note: Some("n".into()) }).unwrap();
    s.backtrack(1_700_000_000_000 + 37 * 5);
    let bytes = s.export();
    let mut copy: Store<Counter> = Store::import(&bytes, Options::default().origin("tab2")).unwrap();
    assert_eq!(copy.events(), s.events());
    assert_eq!(copy.len(), 21);
    assert!(!copy.live());
    assert_eq!(copy.head_ms(), s.head_ms());
    assert_eq!(copy.state(), s.state());
    assert_eq!(copy.export(), bytes);
    copy.go_live();
    assert_eq!(copy.state().count, 210);
    assert_eq!(copy.state().label, "héllo");

    let text = s.export_text();
    let mut again: Store<Counter> = Store::import_text(&text, Options::default()).unwrap();
    assert_eq!(again.events(), s.events());
}

#[test]
fn import_rejects_other_apps_and_garbage() {
    struct Other;
    impl App for Other {
        const NAME: &'static str = "other";
        type Event = Msg;
        type State = State;
        type Effect = Nothing;
        type Outcome = Nothing;
        fn apply(_: &mut State, _: &Msg, _: &mut Ctx<Self>) {}
        fn view(_: &State, _: &Frame) -> Html<Msg> {
            div().build()
        }
    }
    let mut s = store("a", 1000, 10);
    s.dispatch(Msg::Add(1)).unwrap();
    assert!(matches!(Store::<Other>::import(&s.export(), Options::default()), Err(reagera::StoreError::BadSession(_))));
    assert!(Store::<Counter>::import(b"nope", Options::default()).is_err());
    assert!(Store::<Counter>::import_text("***", Options::default()).is_err());
}

#[test]
fn import_with_the_same_origin_continues_the_sequence() {
    let mut s = store("same", 1000, 10);
    s.dispatch(Msg::Add(1)).unwrap();
    let mut copy: Store<Counter> = Store::import(&s.export(), Options::default().origin("same").clock(ticking(2000, 10))).unwrap();
    let r = copy.dispatch(Msg::Add(1)).unwrap();
    assert_eq!(r.seq(), 1);
    assert_eq!(copy.state().count, 2);
}

#[test]
fn fork_is_independent() {
    let mut s = store("a", 1000, 10);
    s.dispatch(Msg::Add(1)).unwrap();
    let mut twin = s.fork(Options::default().origin("b").clock(ticking(2000, 10)));
    twin.dispatch(Msg::Add(41)).unwrap();
    assert_eq!(s.state().count, 1);
    assert_eq!(twin.state().count, 42);
}

#[test]
fn events_between_explains_a_range() {
    let mut s = store("a", 1000, 10);
    for i in 0..5 {
        s.dispatch(Msg::Add(i)).unwrap();
    }
    let bodies: Vec<_> = s.events_between(1010, 1030).iter().map(|r| r.body().clone()).collect();
    assert_eq!(bodies, vec![Body::Event(Msg::Add(2)), Body::Event(Msg::Add(3))]);
    assert!(s.events_between(1030, 1010).is_empty());
    assert_eq!(s.events_between(i64::MIN, i64::MAX).len(), 5);
}

#[test]
fn record_bytes_are_explicit() {
    let r = Record::new(1, "a", 2, Msg::Add(3));
    let bytes = minbin::to_bytes(&r).unwrap();
    // ms i64 | origin len u32 + "a" | seq u32 | tag u8 | n i64
    assert_eq!(bytes, [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, b'a', 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0, 3]);
    let back: Record<Msg> = minbin::from_bytes(&bytes).unwrap();
    assert_eq!(back, r);
    assert_eq!(back.id(), r.id());

    // In a store the body is wrapped: tag u8 (0 event) then the event.
    let mut s = store("a", 1, 1);
    let r = s.dispatch(Msg::Add(3)).unwrap();
    let bytes = minbin::to_bytes(&r).unwrap();
    assert_eq!(bytes, [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, b'a', 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 3]);
}

#[test]
fn views_are_testable_without_a_browser() {
    let mut s = store("a", 1000, 10);
    s.dispatch(Msg::Add(4)).unwrap();
    s.backtrack(1000);
    let html = Counter::view(s.state(), &s.frame());
    assert_eq!(html.to_html_string(), "<div><button>+</button><span>4</span><em>past</em></div>");
    let msgs = html.find("button").unwrap().fire("click", &DomEvent::default());
    assert_eq!(msgs, vec![Msg::Add(1)]);
}
