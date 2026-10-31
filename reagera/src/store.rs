//! The store: a ConTime history of records plus a movable head.
//!
//! ConTime is the only copy of the history. Records are inserted by their own
//! tick, never by arrival order; a late record rebuilds the states after it;
//! checkpoints keep queries cheap; the event list for export is a ConTime
//! query. The store adds what a UI needs on top: an origin and a sequence
//! counter, a monotonic dispatch time, a head that can look into the past,
//! the requests that still wait for an answer, and sessions as minbin bytes.
//!
//! Processing is Manual: no threads. Every operation enqueues work and then
//! drains the worker on the calling thread, so results are available when the
//! method returns, in the browser and in tests alike.

use crate::{
    app::{App, Frame},
    base64,
    effect::{Body, Ctx, Pending, Rec},
    record::ROOT_HISTORY,
    session::Session,
    tick::Tick,
};
use contime::{
    checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot, Timestamp},
    manual::{ManualConfig, RawManualConTime},
};
use std::{cell::RefCell, collections::HashSet, fmt, num::NonZeroUsize, rc::Rc};

/// The ConTime snapshot: the app state, the requests that have no answer yet,
/// and the time both are valid at. Pending requests are part of the snapshot,
/// so they are recomputed with the state after a late record or an import.
pub struct Snap<A: App> {
    time: Tick,
    state: A::State,
    pending: Vec<Pending<A::Effect>>,
}

impl<A: App> Clone for Snap<A> {
    fn clone(&self) -> Self {
        Self { time: self.time, state: self.state.clone(), pending: self.pending.clone() }
    }
}

impl<A: App> Default for Snap<A> {
    fn default() -> Self {
        Self { time: Tick::default(), state: A::State::default(), pending: Vec::new() }
    }
}

impl<A: App> Snapshot for Snap<A> {
    type Time = Tick;
    fn time(&self) -> &Tick {
        &self.time
    }
    fn set_time(&mut self, time: Tick) {
        self.time = time;
    }
}

impl<A: App> ApplyEvents<Rec<A>> for Snap<A> {
    fn create(_snapshot_id: u128, _first: &Rec<A>) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Tick, Rec<A>>) {
        // ConTime hands over one complete tick at a time, in canonical order.
        for record in batch.events {
            let mut ctx = Ctx::new(record);
            match record.body() {
                Body::Event(event) => A::apply(&mut self.state, event, &mut ctx),
                Body::Reply { effect, request, outcome } => {
                    // A reply settles its request once. A second reply to the
                    // same request, or a reply to a request this fold never
                    // emitted, changes nothing.
                    let Some(pos) = self.pending.iter().position(|p| p.id == *effect) else { continue };
                    self.pending.remove(pos);
                    A::resolve(&mut self.state, request, outcome, &mut ctx);
                }
            }
            self.pending.extend(ctx.finish());
        }
    }
}

type Engine<A> = RawManualConTime<Rec<A>, Snap<A>>;

/// How to build a store.
pub struct Options {
    /// Name of this event source. Default: 8 hex characters from the clock and a counter.
    pub origin: Option<String>,
    /// Source of time for `dispatch`. Default: wall clock milliseconds.
    pub clock: Option<Box<dyn FnMut() -> i64>>,
    /// Save a checkpoint every N events. Default 64.
    pub checkpoint_interval: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self { origin: None, clock: None, checkpoint_interval: 64 }
    }
}

impl Options {
    /// Set the origin.
    pub fn origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }
    /// Set the clock.
    pub fn clock(mut self, clock: impl FnMut() -> i64 + 'static) -> Self {
        self.clock = Some(Box::new(clock));
        self
    }
}

/// Why a store operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The bytes are not a session, or not one of this app.
    BadSession(String),
    /// The head is in the past. Time travel is read-only; call `go_live` to write.
    NotLive,
    /// No request with this ID waits for an answer.
    UnknownEffect(u128),
    /// The record does not fit the wire format: a string over 100 KiB or a record over 1 MiB.
    TooLarge(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSession(m) => write!(f, "bad session: {m}"),
            Self::NotLive => write!(f, "the head is in the past; go live before dispatching"),
            Self::UnknownEffect(id) => write!(f, "no pending request #{}", crate::effect::short(*id)),
            Self::TooLarge(m) => write!(f, "record does not fit the wire format: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// What the ConTime worker reports about itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineStatus {
    /// Number of histories the worker owns. One for a Reagera app.
    pub histories: usize,
    /// Histories with work still queued.
    pub pending: usize,
    /// The tick live processing has been asked to reach.
    pub processing_target: Tick,
    /// Checkpoint spacing in events.
    pub checkpoint_interval: u64,
}

/// The history, the head, and the sessions of one app.
pub struct Store<A: App> {
    engine: Engine<A>,
    /// IDs of retained records. An index for `receive`, not a copy of the history.
    ids: HashSet<u128>,
    earliest: Option<Tick>,
    latest: Option<Tick>,
    origin: String,
    seq: u32,
    head_ms: Option<i64>,
    clock: Box<dyn FnMut() -> i64>,
    current: Snap<A>,
    checkpoint_interval: u64,
    rejections: Vec<String>,
}

impl<A: App> Store<A> {
    /// A store with no events.
    pub fn new(options: Options) -> Self {
        let mut clock = options.clock.unwrap_or_else(|| Box::new(wall_clock));
        let origin = options.origin.unwrap_or_else(|| random_origin(clock()));
        Self {
            engine: new_engine(options.checkpoint_interval),
            ids: HashSet::new(),
            earliest: None,
            latest: None,
            origin,
            seq: 0,
            head_ms: None,
            clock,
            current: Snap::default(),
            checkpoint_interval: options.checkpoint_interval,
            rejections: Vec::new(),
        }
    }

    // ----- reading -------------------------------------------------------

    /// This store's origin name.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// True when the head follows the newest event.
    pub fn live(&self) -> bool {
        self.head_ms.is_none()
    }

    /// The millisecond the state is shown at. In live mode, the newest event's.
    pub fn head_ms(&self) -> i64 {
        self.head_ms.unwrap_or_else(|| self.latest_ms())
    }

    /// Millisecond of the newest retained record, or 0 when empty.
    pub fn latest_ms(&self) -> i64 {
        self.latest.map_or(0, |t| t.ms)
    }

    /// Millisecond of the oldest retained record, or 0 when empty.
    pub fn earliest_ms(&self) -> i64 {
        self.earliest.map_or(0, |t| t.ms)
    }

    /// Number of retained records.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when no record is retained.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// State at the head. Kept current by every mutating method.
    pub fn state(&self) -> &A::State {
        &self.current.state
    }

    /// Requests that had no answer as of the head, oldest first. Derived from
    /// the history like the state, so a replayed session shows the same list.
    pub fn pending(&self) -> &[Pending<A::Effect>] {
        &self.current.pending
    }

    /// Requests that have no answer as of the newest record, whatever the head.
    /// This is the list a runtime performs.
    pub fn pending_live(&mut self) -> Vec<Pending<A::Effect>> {
        if self.live() {
            return self.current.pending.clone();
        }
        self.query_snap(self.latest.unwrap_or_default()).pending
    }

    /// What a view may know besides the state.
    pub fn frame(&self) -> Frame {
        Frame { live: self.live(), head_ms: self.head_ms(), events: self.len(), pending: self.current.pending.len() }
    }

    /// Every retained record in canonical order, from the ConTime history.
    pub fn events(&mut self) -> Vec<Rec<A>> {
        self.query_events(Tick::minimum(), Tick::end_of(i64::MAX))
    }

    /// Records with `from_ms < ms <= to_ms`, in canonical order.
    pub fn events_between(&mut self, from_ms: i64, to_ms: i64) -> Vec<Rec<A>> {
        if to_ms <= from_ms {
            return Vec::new();
        }
        self.query_events(Tick { ms: from_ms.saturating_add(1), seq: 0 }, Tick { ms: to_ms.saturating_add(1), seq: 0 })
    }

    /// Records the history refused, as text. Drained by this call.
    pub fn take_rejections(&mut self) -> Vec<String> {
        std::mem::take(&mut self.rejections)
    }

    /// State at any time, including every event in that millisecond. Does not move the head.
    pub fn state_at(&mut self, ms: i64) -> A::State {
        self.query_snap(Tick::end_of(ms)).state
    }

    /// Requests without an answer at any time. Does not move the head.
    pub fn pending_at(&mut self, ms: i64) -> Vec<Pending<A::Effect>> {
        self.query_snap(Tick::end_of(ms)).pending
    }

    /// What the ConTime worker reports.
    pub fn engine_status(&self) -> EngineStatus {
        let s = self.engine.worker_status();
        EngineStatus { histories: s.history_stores, pending: s.pending_histories, processing_target: s.processing_target, checkpoint_interval: self.checkpoint_interval }
    }

    // ----- writing -------------------------------------------------------

    /// Record an event now. Returns the record so callers can share it.
    ///
    /// The tick is the clock's value, but never earlier than the newest record
    /// this store knows: the millisecond catches up, and within that
    /// millisecond the sequence number catches up too, as a Lamport clock. So a
    /// store's own records are ordered as they were made, and a reply to
    /// something it has seen is ordered after it, whoever made it.
    ///
    /// Fails with [`StoreError::NotLive`] while the head is in the past: time
    /// travel is read-only, because other stores may hold the same history.
    pub fn dispatch(&mut self, event: A::Event) -> Result<Rec<A>, StoreError> {
        if !self.live() {
            return Err(StoreError::NotLive);
        }
        let ms = self.next_ms();
        self.dispatch_at(event, ms)
    }

    /// Record an event at an explicit millisecond, regardless of the head and the clock.
    /// Fails with [`StoreError::TooLarge`] when the event does not fit the wire format.
    pub fn dispatch_at(&mut self, event: A::Event, ms: i64) -> Result<Rec<A>, StoreError> {
        self.record(Body::Event(event), ms)
    }

    /// Record the outside world's answer to a pending request. The request is
    /// looked up among the requests without an answer as of the newest record.
    ///
    /// Unlike `dispatch`, this works while the head is in the past: an answer
    /// is not a new branch of the history, it settles something already in it.
    /// The head does not move.
    pub fn reply(&mut self, effect: u128, outcome: A::Outcome) -> Result<Rec<A>, StoreError> {
        let request = self.pending_live().into_iter().find(|p| p.id == effect).ok_or(StoreError::UnknownEffect(effect))?.request;
        let ms = self.next_ms();
        self.record(Body::Reply { effect, request, outcome }, ms)
    }

    /// Insert a record made elsewhere. Returns false when its ID is already known.
    /// A record older than the newest one is placed at its own tick and later state is rebuilt.
    pub fn receive(&mut self, record: Rec<A>) -> bool {
        if !self.insert(record) {
            return false;
        }
        self.refresh();
        true
    }

    /// Show the state as of `ms`. Nothing is deleted.
    pub fn backtrack(&mut self, ms: i64) {
        self.head_ms = Some(ms);
        self.refresh();
    }

    /// Follow the newest event again.
    pub fn go_live(&mut self) {
        self.head_ms = None;
        self.refresh();
    }

    /// An independent store with the same history and head, like an Aurora clone.
    pub fn fork(&mut self, options: Options) -> Self {
        Self::import(&self.export(), options).expect("a store exports a valid session")
    }

    // ----- sessions ------------------------------------------------------

    /// The session as minbin bytes.
    pub fn export(&mut self) -> Vec<u8> {
        let session = Session { name: A::NAME.to_string(), live: self.live(), head_ms: self.head_ms.unwrap_or(0), records: self.events() };
        minbin::to_bytes(&session).expect("session fits the wire format")
    }

    /// The session as base64 text, for a URL or a bug report.
    pub fn export_text(&mut self) -> String {
        base64::encode(&self.export())
    }

    /// Rebuild a store from `export` bytes.
    pub fn import(bytes: &[u8], options: Options) -> Result<Self, StoreError> {
        let session: Session<Body<A::Event, A::Effect, A::Outcome>> = minbin::from_bytes(bytes).map_err(|e| StoreError::BadSession(format!("{e:?}")))?;
        if session.name != A::NAME {
            return Err(StoreError::BadSession(format!("session is for app {:?}, not {:?}", session.name, A::NAME)));
        }
        let mut store = Self::new(options);
        for record in session.records {
            if record.origin() == store.origin && record.seq() >= store.seq {
                store.seq = record.seq() + 1;
            }
            store.insert(record);
        }
        store.head_ms = if session.live { None } else { Some(session.head_ms) };
        store.refresh();
        Ok(store)
    }

    /// Rebuild a store from `export_text`.
    pub fn import_text(text: &str, options: Options) -> Result<Self, StoreError> {
        let bytes = base64::decode(text.trim()).ok_or_else(|| StoreError::BadSession("not base64".into()))?;
        Self::import(&bytes, options)
    }

    // ----- internals -----------------------------------------------------

    /// The clock's millisecond, but never earlier than the newest record this
    /// store knows. Within that millisecond the sequence number catches up too.
    fn next_ms(&mut self) -> i64 {
        let ms = (self.clock)().max(self.latest_ms());
        if let Some(latest) = self.latest {
            if latest.ms == ms && self.seq <= latest.seq {
                self.seq = latest.seq.saturating_add(1);
            }
        }
        ms
    }

    /// Nothing changes, not even the sequence number, when the body does not fit.
    fn record(&mut self, body: Body<A::Event, A::Effect, A::Outcome>, ms: i64) -> Result<Rec<A>, StoreError> {
        let record = Rec::<A>::try_new(ms, self.origin.clone(), self.seq, body).map_err(|e| StoreError::TooLarge(format!("{e:?}")))?;
        self.seq += 1;
        self.insert(record.clone());
        self.refresh();
        Ok(record)
    }

    fn insert(&mut self, record: Rec<A>) -> bool {
        if !self.ids.insert(record.id()) {
            return false;
        }
        let tick = record.tick();
        self.earliest = Some(self.earliest.map_or(tick, |e| e.min(tick)));
        self.latest = Some(self.latest.map_or(tick, |l| l.max(tick)));
        self.engine.apply([record]).expect("manual queue has room");
        self.engine.advance_to(self.latest.unwrap_or_default()).expect("manual queue has room");
        self.drain();
        true
    }

    fn drain(&mut self) {
        loop {
            match self.engine.process() {
                Ok(true) => continue,
                Ok(false) => break,
                Err(e) => {
                    self.rejections.push(format!("engine: {e:?}"));
                    break;
                }
            }
        }
        for r in self.engine.take_rejections() {
            self.rejections.push(format!("{r:?}"));
        }
    }

    fn query_snap(&mut self, tick: Tick) -> Snap<A> {
        let slot: Rc<RefCell<Option<Snap<A>>>> = Rc::new(RefCell::new(None));
        let out = slot.clone();
        self.engine
            .query_at(tick, [ROOT_HISTORY], move |snaps| {
                *out.borrow_mut() = snaps.into_iter().next().map(|s| *s);
            })
            .expect("manual queue has room");
        self.drain();
        let result = slot.borrow_mut().take();
        result.unwrap_or_default()
    }

    /// Records in the half-open tick interval `[from, to)`.
    fn query_events(&mut self, from: Tick, to: Tick) -> Vec<Rec<A>> {
        if self.ids.is_empty() {
            return Vec::new();
        }
        let slot: Rc<RefCell<Vec<Rec<A>>>> = Rc::new(RefCell::new(Vec::new()));
        let out = slot.clone();
        self.engine
            .query_events_between(ROOT_HISTORY, from, to, move |events| {
                *out.borrow_mut() = events.iter().map(|e| (**e).clone()).collect();
            })
            .expect("manual queue has room");
        self.drain();
        let result = std::mem::take(&mut *slot.borrow_mut());
        result
    }

    fn refresh(&mut self) {
        let tick = match self.head_ms {
            Some(ms) => Tick::end_of(ms),
            None => self.latest.unwrap_or_default(),
        };
        self.current = self.query_snap(tick);
    }
}

fn new_engine<A: App>(checkpoint_interval: u64) -> Engine<A> {
    RawManualConTime::new(
        ManualConfig {
            queue_capacity: NonZeroUsize::new(1024).expect("non-zero"),
            checkpoints: CheckpointConfig { interval: checkpoint_interval.max(1) },
        },
        (),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn wall_clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(target_arch = "wasm32")]
fn wall_clock() -> i64 {
    js_sys::Date::now() as i64
}

/// Eight hex digits from the OS or browser random source. The clock is mixed in
/// only as a fallback, so two tabs that start in the same millisecond still differ.
fn random_origin(seed: i64) -> String {
    let mut bytes = [0u8; 4];
    if getrandom::fill(&mut bytes).is_err() {
        let mut x = (seed as u64) ^ 0x9e37_79b9_7f4a_7c15;
        x ^= x >> 30;
        x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x ^= x >> 27;
        bytes = ((x >> 32) as u32).to_be_bytes();
    }
    format!("{:08x}", u32::from_be_bytes(bytes))
}
