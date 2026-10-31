//! Latency lab: two clients share an arena. Each click is a record. Records
//! reach the other client after a random delay, so they often arrive out of
//! order. ConTime inserts a late record at its own tick and rebuilds the states
//! after it. When nothing is in flight, both clients and a reference store agree.

use reagera::{html::*, minbin, App, Ctx, Frame, Html, Nothing};

// ----- the app ---------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Msg {
    #[default]
    Clear,
    Place { x: u16, y: u16 },
}

minbin::minbin_enum! { Msg [
    [0 => Self::Clear],
    [1 => Self::Place { x: u16, y: u16 }]
] }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dot {
    pub id: u128,
    pub x: u16,
    pub y: u16,
    pub n: u32,
    pub origin: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub dots: Vec<Dot>,
    pub placed: u32,
}

/// The lab draws two stores side by side, so the app has no view of its own.
pub struct Arena;

impl App for Arena {
    const NAME: &'static str = "latency-lab/2";
    type Event = Msg;
    type State = State;
    type Effect = Nothing;
    type Outcome = Nothing;

    // ConTime calls this in canonical order and again after a late record, so
    // `n` always shows the order the history settled on.
    fn apply(state: &mut State, event: &Msg, ctx: &mut Ctx<Self>) {
        match event {
            Msg::Place { x, y } => {
                state.placed += 1;
                state.dots.push(Dot { id: ctx.id(), x: *x, y: *y, n: state.placed, origin: ctx.origin().to_string() });
            }
            Msg::Clear => state.dots.clear(),
        }
    }

    fn view(_: &State, _: &Frame) -> Html<Msg> {
        div().build()
    }
}

// ----- the simulation (not part of any log) ------------------------------------

#[cfg(target_arch = "wasm32")]
mod lab {
    use super::*;
    use reagera::web::{every, next_frame, now_ms, Renderer};
    use reagera::Rec;
    use reagera::{Options, Store};
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::prelude::*;

    const W: i32 = 320;
    const H: i32 = 220;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Client {
        Alice,
        Bob,
    }

    impl Client {
        fn name(self) -> &'static str {
            match self {
                Client::Alice => "alice",
                Client::Bob => "bob",
            }
        }
        fn other(self) -> Self {
            match self {
                Client::Alice => Client::Bob,
                Client::Bob => Client::Alice,
            }
        }
    }

    fn colour(origin: &str) -> &'static str {
        match origin {
            "alice" => "#2563eb",
            "bob" => "#dc2626",
            _ => "#6b7280",
        }
    }

    #[derive(Clone, Debug)]
    enum LabMsg {
        Click(Client, i32, i32),
        Latency(i64),
        Jitter(i64),
        ToggleBot,
        DeliverAll,
        Clear,
        Scrub(i64),
        Live,
        ShareUrl,
    }

    struct InFlight {
        to: Client,
        record: Rec<Arena>,
        due_ms: i64,
    }

    struct Lab {
        alice: Store<Arena>,
        bob: Store<Arena>,
        reference: Store<Arena>,
        in_flight: Vec<InFlight>,
        late: Vec<(Client, u128)>,
        latency: i64,
        jitter: i64,
        bot: bool,
        scrub: Option<i64>,
        t0: i64,
        rng: u64,
    }

    impl Lab {
        fn store(&mut self, c: Client) -> &mut Store<Arena> {
            match c {
                Client::Alice => &mut self.alice,
                Client::Bob => &mut self.bob,
            }
        }

        fn rand(&mut self) -> f64 {
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 7;
            self.rng ^= self.rng << 17;
            (self.rng >> 11) as f64 / (1u64 << 53) as f64
        }

        fn emit(&mut self, from: Client, body: Msg) {
            let Ok(record) = self.store(from).dispatch(body) else { return };
            self.reference.receive(record.clone());
            let delay = (self.latency as f64 + (self.rand() * 2.0 - 1.0) * self.jitter as f64).max(0.0) as i64;
            self.in_flight.push(InFlight { to: from.other(), record, due_ms: now_ms() + delay });
        }

        fn deliver_due(&mut self) -> bool {
            let now = now_ms();
            let ready: Vec<InFlight> = {
                let (ready, rest): (Vec<_>, Vec<_>) = self.in_flight.drain(..).partition(|m| m.due_ms <= now);
                self.in_flight = rest;
                ready
            };
            let any = !ready.is_empty();
            for m in ready {
                let is_late = m.record.ms() < self.store(m.to).latest_ms();
                if is_late {
                    self.late.push((m.to, m.record.id()));
                }
                self.store(m.to).receive(m.record);
            }
            any
        }

        fn handle(&mut self, msg: LabMsg) {
            match msg {
                LabMsg::Click(c, x, y) => self.emit(c, Msg::Place { x: x.clamp(0, W) as u16, y: y.clamp(0, H) as u16 }),
                LabMsg::Latency(v) => self.latency = v,
                LabMsg::Jitter(v) => self.jitter = v,
                LabMsg::ToggleBot => self.bot = !self.bot,
                LabMsg::DeliverAll => {
                    for m in &mut self.in_flight {
                        m.due_ms = 0;
                    }
                    self.deliver_due();
                }
                LabMsg::Clear => self.emit(Client::Alice, Msg::Clear),
                LabMsg::Scrub(ms) => self.scrub = Some(ms),
                LabMsg::Live => self.scrub = None,
                LabMsg::ShareUrl => {
                    if let Some(w) = web_sys::window() {
                        let _ = w.location().set_hash(&self.reference.export_text());
                    }
                }
            }
        }

        fn converged(&self) -> bool {
            self.alice.state() == self.bob.state() && self.alice.state() == self.reference.state()
        }

        fn arena(&mut self, c: Client) -> Html<LabMsg> {
            let through = self.scrub.unwrap_or(i64::MAX);
            let state = match self.scrub {
                Some(ms) => self.store(c).state_at(ms),
                None => self.store(c).state().clone(),
            };
            let pending = self.in_flight.iter().filter(|m| m.to == c).count();
            let late: Vec<u128> = self.late.iter().filter(|(lc, _)| *lc == c).map(|(_, id)| *id).collect();
            let t0 = self.t0;
            let events = self.store(c).events();
            section()
                .class("client")
                .child(
                    header()
                        .child(strong().style(format!("color:{}", colour(c.name()))).text(c.name()))
                        .child(span().text(format!("{} events", events.len())))
                        .child(if pending > 0 { span().class("pending").text(format!("{pending} in flight")) } else { span().class("ok").text("nothing in flight") })
                        .child(span().text(format!("{} arrived late", late.len()))),
                )
                .child(
                    svg()
                        .attr("viewBox", format!("0 0 {W} {H}"))
                        .attr("width", W)
                        .attr("height", H)
                        .class("arena")
                        .on("click", move |e| {
                            let x = e.x * W / e.width.max(1);
                            let y = e.y * H / e.height.max(1);
                            Some(LabMsg::Click(c, x, y))
                        })
                        .children(state.dots.iter().map(|d| {
                            g().key(format!("{:x}", d.id))
                                .child(
                                    circle()
                                        .attr("cx", d.x)
                                        .attr("cy", d.y)
                                        .attr("r", 9)
                                        .attr("fill", colour(&d.origin))
                                        .attr("stroke", if late.contains(&d.id) { "#f59e0b" } else { "none" })
                                        .attr("stroke-width", 3),
                                )
                                .child(el("text").attr("x", d.x).attr("y", d.y + 4).attr("text-anchor", "middle").attr("font-size", 10).attr("fill", "#fff").text(d.n.to_string()))
                                .build()
                        })),
                )
                .child(ol().class("log").children(events.iter().rev().take(8).rev().map(|r| {
                    let is_late = late.contains(&r.id());
                    let future = r.ms() > through;
                    li().key(format!("{:x}", r.id()))
                        .class(format!("{}{}", if is_late { "late " } else { "" }, if future { "future" } else { "" }))
                        .text(format!("+{:.2}s {} {:?}", (r.ms() - t0) as f64 / 1000.0, r.origin(), r.body()))
                        .build()
                })))
                .build()
        }

        fn view(&mut self) -> Html<LabMsg> {
            let latest = self.reference.latest_ms();
            let earliest = self.reference.earliest_ms();
            let inflight = self.in_flight.len();
            let converged = self.converged();
            let t0 = self.t0;
            let alice = self.arena(Client::Alice);
            let bob = self.arena(Client::Bob);
            main()
                .child(h1().text("Latency lab"))
                .child(p().class("hint").text(
                    "Click either arena to place a dot. Dots travel to the other client with the delay below. \
                     An orange ring marks a dot that arrived late and was inserted behind newer ones. \
                     The number on each dot is the order the history settles on. Both agree when nothing is in flight.",
                ))
                .child(
                    div()
                        .class("controls")
                        .child(slider("latency", self.latency, LabMsg::Latency))
                        .child(slider("jitter", self.jitter, LabMsg::Jitter))
                        .child(label().child(input().attr("type", "checkbox").checked(self.bot).on("change", |_| Some(LabMsg::ToggleBot))).text(" bob clicks on his own"))
                        .child(button().on_click(LabMsg::DeliverAll).text("deliver everything now"))
                        .child(button().on_click(LabMsg::Clear).text("clear (as alice)"))
                        .child(button().on_click(LabMsg::ShareUrl).text("put session in URL")),
                )
                .child(div().class(if converged { "status ok" } else { "status pending" }).text(if converged {
                    "alice, bob and the reference store agree".to_string()
                } else {
                    format!("histories differ: {inflight} event(s) still in flight")
                }))
                .child(
                    label()
                        .class("scrub")
                        .text(match self.scrub {
                            None => "live".to_string(),
                            Some(ms) => format!("showing +{:.2}s", (ms - t0) as f64 / 1000.0),
                        })
                        .child(
                            input()
                                .attr("type", "range")
                                .attr("min", earliest.min(latest))
                                .attr("max", latest.max(earliest + 1))
                                .attr("step", 10)
                                .value(self.scrub.unwrap_or(latest))
                                .on("input", |e| e.value.parse().ok().map(LabMsg::Scrub)),
                        )
                        .child(button().on_click(LabMsg::Live).text("live")),
                )
                .child(div().class("clients").child(alice).child(bob))
                .child(p().class("hint").text(format!(
                    "reference store: {} events, {} bytes exported. Every record is a minbin-encoded ConTime event. Each arena shows state rebuilt by ConTime, and the scrubber queries it at that time without moving the head.",
                    self.reference.len(),
                    self.reference.export().len()
                )))
                .build()
        }
    }

    fn slider(name: &'static str, value: i64, make: fn(i64) -> LabMsg) -> Html<LabMsg> {
        label()
            .text(format!("{name} {value}ms"))
            .child(input().attr("type", "range").attr("min", 0).attr("max", 3000).attr("step", 50).value(value).on("input", move |e| e.value.parse().ok().map(make)))
            .build()
    }

    #[wasm_bindgen(start)]
    fn start() {
        let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
        let mut reference = match hash.strip_prefix('#').filter(|h| !h.is_empty()) {
            Some(text) => Store::<Arena>::import_text(text, Options::default().origin("reference")).unwrap_or_else(|_| Store::new(Options::default().origin("reference"))),
            None => Store::new(Options::default().origin("reference")),
        };
        let mut alice = Store::<Arena>::new(Options::default().origin("alice"));
        let mut bob = Store::<Arena>::new(Options::default().origin("bob"));
        for r in reference.events() {
            alice.receive(r.clone());
            bob.receive(r);
        }
        let t0 = if reference.is_empty() { now_ms() } else { reference.earliest_ms() };
        let lab = Rc::new(RefCell::new(Lab {
            alice,
            bob,
            reference,
            in_flight: Vec::new(),
            late: Vec::new(),
            latency: 800,
            jitter: 800,
            bot: true,
            scrub: None,
            t0,
            rng: (now_ms() as u64) | 1,
        }));
        let root = reagera::web::document().get_element_by_id("root").expect("#root");
        let renderer: Rc<RefCell<Option<Renderer<LabMsg>>>> = Rc::new(RefCell::new(None));

        let render = {
            let lab = lab.clone();
            let renderer = renderer.clone();
            let scheduled = Rc::new(std::cell::Cell::new(false));
            Rc::new(move || {
                if scheduled.replace(true) {
                    return;
                }
                let lab = lab.clone();
                let renderer = renderer.clone();
                let scheduled = scheduled.clone();
                next_frame(move || {
                    scheduled.set(false);
                    let tree = lab.borrow_mut().view();
                    if let Some(r) = renderer.borrow_mut().as_mut() {
                        r.render(tree);
                    }
                });
            })
        };

        let dispatch: Rc<dyn Fn(LabMsg)> = {
            let lab = lab.clone();
            let render = render.clone();
            Rc::new(move |msg| {
                lab.borrow_mut().handle(msg);
                render();
            })
        };
        let mut first = Renderer::new(root, dispatch);
        first.render(lab.borrow_mut().view());
        *renderer.borrow_mut() = Some(first);

        {
            let lab = lab.clone();
            let render = render.clone();
            every(50, move || {
                if lab.borrow_mut().deliver_due() {
                    render();
                }
            });
        }
        {
            let lab = lab.clone();
            let render = render.clone();
            every(1500, move || {
                let mut l = lab.borrow_mut();
                if l.bot {
                    let x = (l.rand() * W as f64) as u16;
                    let y = (l.rand() * H as f64) as u16;
                    l.emit(Client::Bob, Msg::Place { x, y });
                    drop(l);
                    render();
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reagera::{Options, Store};

    fn store(origin: &str, start: i64) -> Store<Arena> {
        let mut t = start - 100;
        Store::new(Options::default().origin(origin).clock(move || {
            t += 100;
            t
        }))
    }

    #[test]
    fn a_late_record_renumbers_the_dots_and_clients_converge() {
        let mut alice = store("alice", 1000);
        let mut bob = store("bob", 1050);
        let a1 = alice.dispatch(Msg::Place { x: 1, y: 1 }).unwrap(); // 1000
        let a2 = alice.dispatch(Msg::Place { x: 2, y: 2 }).unwrap(); // 1100
        let b1 = bob.dispatch(Msg::Place { x: 3, y: 3 }).unwrap(); // 1050

        assert_eq!(alice.state().dots.iter().map(|d| d.x).collect::<Vec<_>>(), vec![1, 2]);
        alice.receive(b1.clone()); // late: older than a2
        assert_eq!(alice.state().dots.iter().map(|d| (d.x, d.n)).collect::<Vec<_>>(), vec![(1, 1), (3, 2), (2, 3)]);

        bob.receive(a2); // newest first
        bob.receive(a1);
        assert_eq!(alice.state(), bob.state());
    }

    #[test]
    fn clear_applies_at_its_own_time() {
        let mut alice = store("alice", 1000);
        let mut bob = store("bob", 1150);
        alice.dispatch(Msg::Place { x: 1, y: 1 }).unwrap(); // 1000
        let clear = alice.dispatch(Msg::Clear).unwrap(); // 1100
        let b1 = bob.dispatch(Msg::Place { x: 2, y: 2 }).unwrap(); // 1150
        alice.receive(b1);
        bob.receive(clear);
        assert_eq!(alice.state().dots.iter().map(|d| d.x).collect::<Vec<_>>(), vec![2]);
        assert_eq!(bob.state().dots.iter().map(|d| d.x).collect::<Vec<_>>(), vec![2]);
        assert_eq!(alice.state_at(1000).dots.len(), 1);
    }
}
