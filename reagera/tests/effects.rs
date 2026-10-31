//! Effects: requests emitted by the fold, answers recorded in the log, replay
//! that never performs anything twice.

use reagera::{html::*, minbin, App, Body, Ctx, Done, Html, Options, Pending, Rec, Record, Store, StoreError};
use std::{cell::RefCell, rc::Rc};

// A shop: add to the cart, pay. Paying asks a gateway to charge the cart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Msg {
    #[default]
    Pay,
    Add(u32),
    Refund(u128),
}
minbin::minbin_enum! { Msg [[0 => Self::Pay], [1 => Self::Add(cents: u32)], [2 => Self::Refund(receipt: u128)]] }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Effect {
    #[default]
    Noop,
    Charge(u32),
    Email(String),
    Refund(u128),
}
minbin::minbin_enum! { Effect [[0 => Self::Noop], [1 => Self::Charge(cents: u32)], [2 => Self::Email(to: String)], [3 => Self::Refund(receipt: u128)]] }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Outcome {
    #[default]
    Ok,
    Charged(u128),
    Declined(String),
}
minbin::minbin_enum! { Outcome [[0 => Self::Ok], [1 => Self::Charged(receipt: u128)], [2 => Self::Declined(reason: String)]] }

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    cart: u32,
    paying: Option<u128>,
    receipts: Vec<u128>,
    declined: Vec<String>,
    emails: u32,
    log: Vec<String>,
}

struct Shop;

impl App for Shop {
    const NAME: &'static str = "shop-test";
    type Event = Msg;
    type State = State;
    type Effect = Effect;
    type Outcome = Outcome;

    fn apply(state: &mut State, event: &Msg, ctx: &mut Ctx<Self>) {
        state.log.push(format!("{event:?}@{}", ctx.ms()));
        match event {
            Msg::Add(c) => state.cart += c,
            Msg::Pay => {
                // The guard that a React app would put in a ref: one charge at a time.
                if state.cart > 0 && state.paying.is_none() {
                    state.paying = Some(ctx.emit(Effect::Charge(state.cart)));
                }
            }
            Msg::Refund(receipt) => {
                ctx.emit(Effect::Refund(*receipt));
            }
        }
    }

    fn resolve(state: &mut State, request: &Effect, outcome: &Outcome, ctx: &mut Ctx<Self>) {
        state.log.push(format!("{request:?}={outcome:?}@{}", ctx.ms()));
        match (request, outcome) {
            (Effect::Charge(_), Outcome::Charged(receipt)) => {
                state.receipts.push(*receipt);
                state.cart = 0;
                state.paying = None;
                // Chain: a receipt mail, emitted from the reply record.
                ctx.emit(Effect::Email(format!("receipt {receipt:x}")));
            }
            (Effect::Charge(_), Outcome::Declined(why)) => {
                state.declined.push(why.clone());
                state.paying = None;
            }
            (Effect::Email(_), _) => state.emails += 1,
            _ => {}
        }
    }

    // A test performer: answers immediately into a shared box, and counts calls.
    fn perform(pending: &Pending<Effect>, done: Done<Outcome>) {
        PERFORMED.with(|p| p.borrow_mut().push(pending.request.clone()));
        done(match pending.request {
            Effect::Charge(_) => Outcome::Charged(0),
            _ => Outcome::Ok,
        });
    }

    fn view(state: &State, _: &reagera::Frame) -> Html<Msg> {
        div().text(format!("{} in cart", state.cart)).build()
    }
}

thread_local! {
    static PERFORMED: RefCell<Vec<Effect>> = const { RefCell::new(Vec::new()) };
}

fn ticking(start: i64, step: i64) -> impl FnMut() -> i64 {
    let t = Rc::new(std::cell::Cell::new(start - step));
    move || {
        t.set(t.get() + step);
        t.get()
    }
}

fn shop(origin: &str, start: i64) -> Store<Shop> {
    Store::new(Options::default().origin(origin).clock(ticking(start, 10)))
}

/// Do what the browser runtime does: perform every pending request of this origin.
fn run_performer(store: &mut Store<Shop>) -> usize {
    let mine = store.origin().to_string();
    let pending: Vec<_> = store.pending_live().into_iter().filter(|p| p.origin == mine).collect();
    let n = pending.len();
    for p in pending {
        let slot: Rc<RefCell<Option<Outcome>>> = Rc::new(RefCell::new(None));
        let out = slot.clone();
        Shop::perform(&p, Box::new(move |o| *out.borrow_mut() = Some(o)));
        let outcome = slot.borrow_mut().take();
        if let Some(o) = outcome {
            store.reply(p.id, o).unwrap();
        }
    }
    n
}

#[test]
fn apply_emits_a_request_and_the_reply_settles_it() {
    let mut s = shop("a", 1000);
    s.dispatch(Msg::Add(450)).unwrap();
    let pay = s.dispatch(Msg::Pay).unwrap();
    assert_eq!(s.pending().len(), 1);
    let p = s.pending()[0].clone();
    assert_eq!(p.request, Effect::Charge(450));
    assert_eq!(p.origin, "a");
    assert_eq!(p.ms, pay.ms());
    assert_eq!(p.id, reagera::effect::effect_id(pay.id(), 0));
    assert_eq!(s.state().paying, Some(p.id));
    assert_eq!(s.frame().pending, 1);

    let reply = s.reply(p.id, Outcome::Charged(0xabc)).unwrap();
    assert_eq!(reply.body(), &Body::Reply { effect: p.id, request: Effect::Charge(450), outcome: Outcome::Charged(0xabc) });
    assert_eq!(reply.ms(), 1020, "a reply is an ordinary record at the clock's time");
    assert_eq!(s.state().receipts, vec![0xabc]);
    assert_eq!(s.state().cart, 0);
    assert_eq!(s.state().paying, None);
    // The reply emitted a chained request from its own record ID.
    assert_eq!(s.pending().len(), 1);
    assert_eq!(s.pending()[0].request, Effect::Email("receipt abc".into()));
    assert_eq!(s.pending()[0].id, reagera::effect::effect_id(reply.id(), 0));
}

#[test]
fn a_request_is_performed_once_and_never_on_replay() {
    PERFORMED.with(|p| p.borrow_mut().clear());
    let mut s = shop("tab", 1000);
    s.dispatch(Msg::Add(300)).unwrap();
    s.dispatch(Msg::Pay).unwrap();
    assert_eq!(run_performer(&mut s), 1, "the charge");
    assert_eq!(run_performer(&mut s), 1, "the receipt mail, chained from the reply");
    assert_eq!(run_performer(&mut s), 0, "nothing left");
    assert_eq!(PERFORMED.with(|p| p.borrow().clone()), vec![Effect::Charge(300), Effect::Email("receipt 0".into())]);
    assert!(s.pending().is_empty());

    // A bug report: the session text. Replaying it in a fresh store performs nothing.
    PERFORMED.with(|p| p.borrow_mut().clear());
    let text = s.export_text();
    let mut copy = Store::<Shop>::import_text(&text, Options::default().origin("ci")).unwrap();
    assert_eq!(copy.state(), s.state(), "the same answers, the same state");
    assert!(copy.pending().is_empty(), "every request in the log has its answer in the log");
    assert_eq!(run_performer(&mut copy), 0);
    assert!(PERFORMED.with(|p| p.borrow().is_empty()), "replay performed nothing");
    assert_eq!(copy.events().len(), 4, "add, pay, charged, mailed");
}

#[test]
fn a_pending_request_of_another_origin_is_not_performed_here() {
    PERFORMED.with(|p| p.borrow_mut().clear());
    let mut s = shop("reporter", 1000);
    s.dispatch(Msg::Add(1)).unwrap();
    s.dispatch(Msg::Pay).unwrap();
    // The reporter exported before the gateway answered.
    let mut copy = Store::<Shop>::import(&s.export(), Options::default().origin("dev")).unwrap();
    assert_eq!(copy.pending().len(), 1, "the unanswered charge is visible");
    assert_eq!(copy.pending()[0].origin, "reporter");
    assert_eq!(run_performer(&mut copy), 0, "and it is not ours to perform");
    // The developer can still supply the answer by hand to continue the story.
    let id = copy.pending()[0].id;
    copy.reply(id, Outcome::Declined("insufficient funds".into())).unwrap();
    assert_eq!(copy.state().declined, vec!["insufficient funds"]);
    assert!(copy.pending().is_empty());
}

#[test]
fn double_pay_charges_once() {
    let mut s = shop("a", 1000);
    s.dispatch(Msg::Add(100)).unwrap();
    s.dispatch(Msg::Pay).unwrap();
    s.dispatch(Msg::Pay).unwrap(); // the user clicked twice
    assert_eq!(s.pending().len(), 1);
    let id = s.pending()[0].id;
    s.reply(id, Outcome::Charged(1)).unwrap();
    s.dispatch(Msg::Pay).unwrap(); // nothing in the cart: no charge
    assert_eq!(s.pending().iter().filter(|p| matches!(p.request, Effect::Charge(_))).count(), 0);
}

#[test]
fn a_reply_to_an_unknown_request_is_refused() {
    let mut s = shop("a", 1000);
    assert_eq!(s.reply(42, Outcome::Ok), Err(StoreError::UnknownEffect(42)));
    s.dispatch(Msg::Add(5)).unwrap();
    s.dispatch(Msg::Pay).unwrap();
    let id = s.pending()[0].id;
    s.reply(id, Outcome::Charged(9)).unwrap();
    assert_eq!(s.reply(id, Outcome::Charged(10)), Err(StoreError::UnknownEffect(id)), "answered once");
    assert_eq!(s.state().receipts, vec![9]);
}

#[test]
fn a_second_answer_in_the_log_changes_nothing() {
    // Two clients answer the same request, or one answers twice after a retry.
    // Both records are retained, but the fold settles the request once.
    let mut s = shop("a", 1000);
    s.dispatch(Msg::Add(5)).unwrap();
    s.dispatch(Msg::Pay).unwrap();
    let id = s.pending()[0].id;
    let first = s.reply(id, Outcome::Charged(1)).unwrap();
    let second: Rec<Shop> = Record::new(first.ms() + 1, "b", 0, Body::Reply { effect: id, request: Effect::Charge(5), outcome: Outcome::Charged(2) });
    assert!(s.receive(second));
    assert_eq!(s.events().len(), 4);
    assert_eq!(s.state().receipts, vec![1]);
    assert_eq!(s.state().emails, 0);
    assert_eq!(s.pending().len(), 1, "one mail request, from the first reply only");
}

#[test]
fn a_reply_is_accepted_while_the_head_is_in_the_past() {
    let mut s = shop("a", 1000);
    s.dispatch(Msg::Add(5)).unwrap(); // 1000
    s.dispatch(Msg::Pay).unwrap(); // 1010
    let id = s.pending()[0].id;
    s.backtrack(1000);
    assert_eq!(s.dispatch(Msg::Add(1)), Err(StoreError::NotLive));
    assert!(s.pending().is_empty(), "as of 1000 nothing was requested");
    assert_eq!(s.pending_live().len(), 1, "but live, the charge waits");
    let r = s.reply(id, Outcome::Charged(7)).unwrap();
    assert_eq!(r.ms(), 1020);
    assert!(!s.live(), "the head did not move");
    assert_eq!(s.state().cart, 5, "the head still shows 1000");
    s.go_live();
    assert_eq!(s.state().receipts, vec![7]);
}

#[test]
fn pending_is_a_view_of_the_history_like_the_state() {
    let mut s = shop("a", 1000);
    s.dispatch(Msg::Add(5)).unwrap(); // 1000
    s.dispatch(Msg::Pay).unwrap(); // 1010
    let id = s.pending()[0].id;
    s.reply(id, Outcome::Charged(7)).unwrap(); // 1020
    assert_eq!(s.pending_at(1005).len(), 0);
    assert_eq!(s.pending_at(1010).len(), 1);
    assert_eq!(s.pending_at(1010)[0].request, Effect::Charge(5));
    assert_eq!(s.pending_at(1020).len(), 1, "the mail request");
    s.backtrack(1010);
    assert_eq!(s.pending().len(), 1);
    assert_eq!(s.frame().pending, 1);
    assert_eq!(s.state().paying, Some(id));
}

#[test]
fn effect_ids_agree_across_stores_and_late_records_keep_them() {
    let mut a = shop("a", 1000);
    let mut b = shop("b", 1050);
    a.dispatch(Msg::Add(5)).unwrap(); // 1000
    let pay = a.dispatch(Msg::Pay).unwrap(); // 1010
    let id = a.pending()[0].id;
    let charged = a.reply(id, Outcome::Charged(3)).unwrap(); // 1020
    let late = b.dispatch(Msg::Add(2)).unwrap(); // 1050, lands after everything at a

    // b receives a's records in a scrambled order.
    for r in [charged.clone(), pay.clone(), a.events()[0].clone()] {
        b.receive(r);
    }
    assert_eq!(b.state().receipts, vec![3]);
    assert_eq!(b.pending().len(), 1, "the mail, owned by a");
    assert_eq!(b.pending()[0].origin, "a");
    assert_eq!(b.pending()[0].id, a.pending()[0].id, "same record, same request ID");

    // a receives b's record, which is older than its own mail request's record? No:
    // it is newer (1050 > 1020), so it lands last. Then a late one from before the pay.
    a.receive(late);
    let early: Rec<Shop> = Record::new(1005, "b", 1, Body::Event(Msg::Add(100)));
    a.receive(early);
    // The charge was for 5 cents when a folded alone. With the late add, the
    // fold's Pay sees 105 in the cart and emits Charge(105), which has the same ID
    // (same record, same index). The recorded reply still settles it.
    assert_eq!(a.state().receipts, vec![3]);
    assert_eq!(a.state().cart, 2, "b's 1050 add came after the charge cleared the cart");
    assert_eq!(a.pending().len(), 1, "still just the mail");
}

#[test]
fn sessions_carry_replies_and_rebuild_pending() {
    let mut s = shop("a", 1000);
    s.dispatch(Msg::Add(5)).unwrap();
    s.dispatch(Msg::Pay).unwrap();
    let id = s.pending()[0].id;
    s.reply(id, Outcome::Declined("card expired".into())).unwrap();
    s.dispatch(Msg::Pay).unwrap(); // retry: a new request
    let bytes = s.export();
    let mut copy = Store::<Shop>::import(&bytes, Options::default().origin("x")).unwrap();
    assert_eq!(copy.events(), s.events());
    assert_eq!(copy.state(), s.state());
    assert_eq!(copy.pending(), s.pending());
    assert_eq!(copy.export(), bytes);
    assert_eq!(copy.state().declined, vec!["card expired"]);
    assert_eq!(copy.pending()[0].request, Effect::Charge(5));
}

#[test]
fn reply_bytes_are_explicit() {
    let r: Rec<Shop> = Record::new(1, "a", 0, Body::Reply { effect: 2, request: Effect::Charge(3), outcome: Outcome::Charged(4) });
    let bytes = minbin::to_bytes(&r).unwrap();
    let mut want = vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, b'a', 0, 0, 0, 0]; // ms, origin, seq
    want.push(1); // body tag: reply
    want.extend([0u8; 15]);
    want.push(2); // effect id u128
    want.extend([1, 0, 0, 0, 3]); // request: tag 1, u32 3
    want.push(1); // outcome tag
    want.extend([0u8; 15]);
    want.push(4); // receipt u128
    assert_eq!(bytes, want);
    let back: Rec<Shop> = minbin::from_bytes(&bytes).unwrap();
    assert_eq!(back, r);
    assert_eq!(format!("{:?}", r.body()), "reply #00000000 Charge(3) = Charged(4)");
}

#[test]
fn batch_replays_replies_and_pending_requests_in_log_order() {
    let mut source = shop("a", 1000);
    source.dispatch(Msg::Add(450)).unwrap();
    source.dispatch(Msg::Pay).unwrap();
    let request = source.pending()[0].id;
    source.reply(request, Outcome::Charged(123)).unwrap();
    let records = source.events();
    let mut target = shop("b", 1000);
    assert_eq!(target.receive_batch(records.into_iter().rev()), 3);
    assert_eq!(target.state(), source.state());
    assert_eq!(target.pending(), source.pending());
    assert_eq!(target.pending_at(1010), source.pending_at(1010));
    assert_eq!(target.export(), source.export());
    assert!(target.take_rejections().is_empty());
}
