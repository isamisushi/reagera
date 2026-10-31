//! Checkout: a cart and a payment gateway that is slow and sometimes declines.
//!
//! The gateway is the outside world. `apply` never calls it: pressing "pay"
//! emits a `Charge` request, the runtime hands it to `perform` once, and the
//! answer comes back as a record. Export the session while a charge is in
//! flight, or after a decline, and the importer sees exactly what you saw,
//! without charging anyone again.

use reagera::{html::*, minbin, App, Ctx, Done, Frame, Html, Pending};

// ----- the wire format: events, requests, answers ----------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Msg {
    #[default]
    Pay,
    Add { name: String, cents: u32 },
    Remove(u128),
    Dismiss,
}

minbin::minbin_enum! { Msg [
    [0 => Self::Pay],
    [1 => Self::Add { name: String, cents: u32 }],
    [2 => Self::Remove(id: u128)],
    [3 => Self::Dismiss]
] }

/// What the app asks the outside world for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Effect {
    #[default]
    Nothing,
    Charge { cents: u32, items: u32 },
}

minbin::minbin_enum! { Effect [
    [0 => Self::Nothing],
    [1 => Self::Charge { cents: u32, items: u32 }]
] }

/// What the outside world answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    #[default]
    Nothing,
    Charged { receipt: String },
    Declined { reason: String },
}

minbin::minbin_enum! { Outcome [
    [0 => Self::Nothing],
    [1 => Self::Charged { receipt: String }],
    [2 => Self::Declined { reason: String }]
] }

// ----- the state and the pure rules --------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub id: u128,
    pub name: String,
    pub cents: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Receipt {
    pub receipt: String,
    pub cents: u32,
    pub ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub cart: Vec<Item>,
    /// The charge in flight, if any. One at a time: the guard that stops a double click.
    pub paying: Option<u128>,
    pub receipts: Vec<Receipt>,
    pub declined: Option<String>,
}

impl State {
    pub fn total(&self) -> u32 {
        self.cart.iter().map(|i| i.cents).sum()
    }
}

pub struct Checkout;

impl App for Checkout {
    const NAME: &'static str = "checkout/1";
    type Event = Msg;
    type State = State;
    type Effect = Effect;
    type Outcome = Outcome;

    fn apply(state: &mut State, event: &Msg, ctx: &mut Ctx<Self>) {
        match event {
            Msg::Add { name, cents } => state.cart.push(Item { id: ctx.id(), name: name.clone(), cents: *cents }),
            Msg::Remove(id) => state.cart.retain(|i| i.id != *id),
            Msg::Pay => {
                if !state.cart.is_empty() && state.paying.is_none() {
                    state.declined = None;
                    state.paying = Some(ctx.emit(Effect::Charge { cents: state.total(), items: state.cart.len() as u32 }));
                }
            }
            Msg::Dismiss => state.declined = None,
        }
    }

    fn resolve(state: &mut State, request: &Effect, outcome: &Outcome, ctx: &mut Ctx<Self>) {
        let Effect::Charge { cents, .. } = request else { return };
        state.paying = None;
        match outcome {
            Outcome::Charged { receipt } => {
                state.receipts.push(Receipt { receipt: receipt.clone(), cents: *cents, ms: ctx.ms() });
                state.cart.clear();
            }
            Outcome::Declined { reason } => state.declined = Some(reason.clone()),
            Outcome::Nothing => {}
        }
    }

    /// The only place that touches the outside world. Never called on replay.
    fn perform(pending: &Pending<Effect>, done: Done<Outcome>) {
        #[cfg(target_arch = "wasm32")]
        gateway::charge(pending, done);
        #[cfg(not(target_arch = "wasm32"))]
        let _ = (pending, done);
    }

    fn view(state: &State, frame: &Frame) -> Html<Msg> {
        let paying = state.paying.is_some();
        main()
            .class("app")
            .child(h1().text("checkout"))
            .child(div().class("shelf").children(PRODUCTS.iter().map(|(name, cents)| {
                let (name, cents) = (name.to_string(), *cents);
                button().on_click(Msg::Add { name: name.clone(), cents }).text(format!("+ {name} {}", money(cents))).build()
            })))
            .child(ul().class("cart").children(state.cart.iter().map(|i| {
                let id = i.id;
                li().key(format!("{id:x}"))
                    .child(span().text(&i.name))
                    .child(span().class("price").text(money(i.cents)))
                    .child(button().class("x").on_click(Msg::Remove(id)).text("×"))
                    .build()
            })))
            .child(
                div()
                    .class("pay")
                    .child(strong().text(format!("total {}", money(state.total()))))
                    .child(if paying {
                        button().attr("disabled", "").text("charging…").build()
                    } else {
                        button().class("primary").on_click(Msg::Pay).text("pay").build()
                    }),
            )
            .child_if(state.declined.is_some(), || {
                div().class("declined").text(format!("declined: {}", state.declined.clone().unwrap_or_default())).child(button().on_click(Msg::Dismiss).text("ok")).build()
            })
            .child_if(!state.receipts.is_empty(), || {
                div().class("receipts").child(h2().text("receipts")).children(state.receipts.iter().map(|r| p().key(&r.receipt).text(format!("{} {}", r.receipt, money(r.cents))).build())).build()
            })
            .child_if(!frame.live, || em().class("past").text(format!("viewing the past, {} request(s) were in flight", frame.pending)).build())
            .build()
    }
}

const PRODUCTS: &[(&str, u32)] = &[("coffee", 450), ("bagel", 300), ("book", 1800)];

fn money(cents: u32) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

// ----- the outside world: a fake gateway, slow and unreliable -----------------

#[cfg(target_arch = "wasm32")]
mod gateway {
    use super::{Effect, Outcome};
    use reagera::{web, Done, Pending};

    /// Answers after 0.4 to 2.4 seconds. Declines 3 times in 10. Every receipt
    /// number is random. None of this is in `apply`, so none of it is replayed:
    /// the answer is recorded, and the recording is what a replay sees.
    ///
    /// The request ID goes to the gateway as the idempotency key. A reload
    /// while a charge is in flight keeps the tab's origin, so the runtime
    /// performs the request again; a real gateway answers the same key with
    /// the first charge's result instead of charging twice.
    pub fn charge(pending: &Pending<Effect>, done: Done<Outcome>) {
        let Effect::Charge { cents, .. } = pending.request else { return };
        let delay = 400 + (web::random() * 2000.0) as i32;
        let roll = web::random();
        let receipt = format!("rcpt-{:06x}", (web::random() * 16_777_216.0) as u32);
        web::log(&format!("gateway: charging {cents} cents, idempotency key {:032x}, answer in {delay}ms", pending.id));
        web::after(delay, move || {
            done(if roll < 0.3 { Outcome::Declined { reason: ["card expired", "insufficient funds", "gateway timeout"][(roll * 10.0) as usize % 3].to_string() } } else { Outcome::Charged { receipt } })
        });
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
fn start() {
    use reagera::web::{mount, MountOptions};
    let rt = mount::<Checkout>("root", MountOptions { devtools: true, session_in_url: true, origin: None });
    std::mem::forget(rt);
}

#[cfg(test)]
mod tests {
    use super::*;
    use reagera::{Body, Options, Store};

    fn shop() -> Store<Checkout> {
        let mut t = 1000;
        Store::new(Options::default().origin("tab").clock(move || {
            t += 10;
            t
        }))
    }

    #[test]
    fn pay_asks_the_gateway_once_even_when_clicked_twice() {
        let mut s = shop();
        s.dispatch(Msg::Add { name: "coffee".into(), cents: 450 }).unwrap();
        s.dispatch(Msg::Pay).unwrap();
        s.dispatch(Msg::Pay).unwrap();
        assert_eq!(s.pending().len(), 1);
        assert_eq!(s.pending()[0].request, Effect::Charge { cents: 450, items: 1 });
        let html = Checkout::view(s.state(), &s.frame());
        assert!(html.to_html_string().contains("charging…"));
    }

    #[test]
    fn the_answer_is_a_record_and_a_replay_charges_nobody() {
        let mut s = shop();
        s.dispatch(Msg::Add { name: "book".into(), cents: 1800 }).unwrap();
        s.dispatch(Msg::Pay).unwrap();
        let id = s.pending()[0].id;
        s.reply(id, Outcome::Declined { reason: "card expired".into() }).unwrap();
        assert_eq!(s.state().declined.as_deref(), Some("card expired"));
        s.dispatch(Msg::Pay).unwrap();
        let id = s.pending()[0].id;
        s.reply(id, Outcome::Charged { receipt: "rcpt-1".into() }).unwrap();
        assert!(s.state().cart.is_empty());
        assert_eq!(s.state().receipts[0].receipt, "rcpt-1");

        // The bug report is the session. Nothing is pending in it, so a
        // runtime that imports it has nothing to perform.
        let text = s.export_text();
        let mut copy = Store::<Checkout>::import_text(&text, Options::default()).unwrap();
        assert_eq!(copy.state(), s.state());
        assert!(copy.pending().is_empty());
        let replies = copy.events().iter().filter(|r| matches!(r.body(), Body::Reply { .. })).count();
        assert_eq!(replies, 2);
    }
}
