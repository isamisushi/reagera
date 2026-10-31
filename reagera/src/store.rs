//! The store: a ConTime history of records plus a movable head.
//!
//! ConTime does the hard part. Records are inserted by their own tick, never by
//! arrival order; a late record rebuilds the states after it; checkpoints keep
//! queries cheap. The store adds what a UI needs on top: a journal in canonical
//! order for export, an origin and sequence counter, a head that can look into
//! the past, and sessions as minbin bytes.
//!
//! Processing is Manual: no threads. Every operation enqueues work and then
//! drains the worker on the calling thread, so results are available when the
//! method returns, in the browser and in tests alike.

use crate::{
    app::{App, Frame},
    base64,
    record::{Record, ROOT_HISTORY},
    session::Session,
    tick::Tick,
};
use contime::{
    checkpoints::{ApplyBatch, ApplyEvents, CheckpointConfig, Snapshot},
    manual::{ManualConfig, RawManualConTime},
};
use std::{cell::RefCell, collections::HashSet, fmt, num::NonZeroUsize, rc::Rc};

/// The ConTime snapshot: the app state plus the time it is valid at.
pub struct Snap<A: App> {
    time: Tick,
    state: A::State,
}

impl<A: App> Clone for Snap<A> {
    fn clone(&self) -> Self {
        Self { time: self.time, state: self.state.clone() }
    }
}

impl<A: App> Default for Snap<A> {
    fn default() -> Self {
        Self { time: Tick::default(), state: A::State::default() }
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

impl<A: App> ApplyEvents<Record<A::Event>> for Snap<A> {
    fn create(_snapshot_id: u128, _first: &Record<A::Event>) -> Self {
        Self::default()
    }

    fn apply_events(&mut self, batch: ApplyBatch<'_, '_, Tick, Record<A::Event>>) {
        // ConTime hands over one complete tick at a time, in canonical order.
        for record in batch.events {
            A::apply(&mut self.state, record.body(), record.ms());
        }
    }
}

type Engine<A> = RawManualConTime<Record<<A as App>::Event>, Snap<A>>;

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
    /// A record was rejected by the history.
    Rejected(String),
    /// The ConTime worker failed.
    Engine(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSession(m) => write!(f, "bad session: {m}"),
            Self::Rejected(m) => write!(f, "rejected: {m}"),
            Self::Engine(m) => write!(f, "engine: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// The history, the head, and the sessions of one app.
pub struct Store<A: App> {
    engine: Engine<A>,
    /// Every retained record, in canonical (tick, id) order.
    journal: Vec<Record<A::Event>>,
    ids: HashSet<u128>,
    origin: String,
    seq: u32,
    head_ms: Option<i64>,
    clock: Box<dyn FnMut() -> i64>,
    current: A::State,
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
            journal: Vec::new(),
            ids: HashSet::new(),
            origin,
            seq: 0,
            head_ms: None,
            clock,
            current: A::State::default(),
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
        self.journal.last().map_or(0, Record::ms)
    }

    /// Millisecond of the oldest retained record, or 0 when empty.
    pub fn earliest_ms(&self) -> i64 {
        self.journal.first().map_or(0, Record::ms)
    }

    /// State at the head. Kept current by every mutating method.
    pub fn state(&self) -> &A::State {
        &self.current
    }

    /// What a view may know besides the state.
    pub fn frame(&self) -> Frame {
        Frame { live: self.live(), head_ms: self.head_ms(), events: self.journal.len() }
    }

    /// Every retained record, oldest first.
    pub fn events(&self) -> &[Record<A::Event>] {
        &self.journal
    }

    /// Records with `from_ms < ms <= to_ms`.
    pub fn events_between(&self, from_ms: i64, to_ms: i64) -> &[Record<A::Event>] {
        let start = self.journal.partition_point(|r| r.ms() <= from_ms);
        let end = self.journal.partition_point(|r| r.ms() <= to_ms);
        &self.journal[start..end.max(start)]
    }

    /// Records the history refused, as text. Drained by this call.
    pub fn take_rejections(&mut self) -> Vec<String> {
        std::mem::take(&mut self.rejections)
    }

    /// State at any time, including every event in that millisecond. Does not move the head.
    pub fn state_at(&mut self, ms: i64) -> A::State {
        self.query(Tick::end_of(ms))
    }

    // ----- writing -------------------------------------------------------

    /// Record an event at the clock's time. Returns the record so callers can share it.
    ///
    /// When the head is in the past, every record after the head is dropped
    /// first; the new record starts a new future, as a rewind would.
    pub fn dispatch(&mut self, body: A::Event) -> Record<A::Event> {
        self.dispatch_at(body, None)
    }

    /// Record an event at an explicit millisecond.
    pub fn dispatch_at(&mut self, body: A::Event, ms: Option<i64>) -> Record<A::Event> {
        if let Some(head) = self.head_ms {
            self.truncate_after(head);
        }
        let ms = ms.unwrap_or_else(|| (self.clock)());
        let record = Record::new(ms, self.origin.clone(), self.seq, body);
        self.seq += 1;
        self.insert(record.clone());
        self.refresh();
        record
    }

    /// Insert a record made elsewhere. Returns false when its ID is already known.
    /// A record older than the newest one is placed at its own tick and later state is rebuilt.
    pub fn receive(&mut self, record: Record<A::Event>) -> bool {
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
    pub fn fork(&self, options: Options) -> Self {
        Self::import(&self.export(), options).expect("a store exports a valid session")
    }

    // ----- sessions ------------------------------------------------------

    /// The session as minbin bytes.
    pub fn export(&self) -> Vec<u8> {
        let session = Session {
            name: A::NAME.to_string(),
            live: self.live(),
            head_ms: self.head_ms.unwrap_or(0),
            records: self.journal.clone(),
        };
        minbin::to_bytes(&session).expect("session fits the wire format")
    }

    /// The session as base64 text, for a URL or a bug report.
    pub fn export_text(&self) -> String {
        base64::encode(&self.export())
    }

    /// Rebuild a store from `export` bytes.
    pub fn import(bytes: &[u8], options: Options) -> Result<Self, StoreError> {
        let session: Session<A::Event> = minbin::from_bytes(bytes).map_err(|e| StoreError::BadSession(format!("{e:?}")))?;
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

    fn insert(&mut self, record: Record<A::Event>) -> bool {
        if !self.ids.insert(record.id()) {
            return false;
        }
        let key = (record.tick(), record.id());
        let pos = self.journal.partition_point(|r| (r.tick(), r.id()) < key);
        self.engine.apply([record.clone()]).expect("manual queue has room");
        self.journal.insert(pos, record);
        let latest = self.journal.last().map(Record::tick).unwrap_or_default();
        self.engine.advance_to(latest).expect("manual queue has room");
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

    fn query(&mut self, tick: Tick) -> A::State {
        let slot: Rc<RefCell<Option<A::State>>> = Rc::new(RefCell::new(None));
        let out = slot.clone();
        self.engine
            .query_at(tick, [ROOT_HISTORY], move |snaps| {
                *out.borrow_mut() = snaps.into_iter().next().map(|s| s.state);
            })
            .expect("manual queue has room");
        self.drain();
        let result = slot.borrow_mut().take();
        result.unwrap_or_default()
    }

    fn refresh(&mut self) {
        let tick = match self.head_ms {
            Some(ms) => Tick::end_of(ms),
            None => self.journal.last().map(Record::tick).unwrap_or_default(),
        };
        self.current = self.query(tick);
    }

    fn truncate_after(&mut self, head_ms: i64) {
        let keep: Vec<_> = self.journal.iter().filter(|r| r.ms() <= head_ms).cloned().collect();
        self.engine = new_engine(self.checkpoint_interval);
        self.journal.clear();
        self.ids.clear();
        self.head_ms = None;
        for record in keep {
            self.insert(record);
        }
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

fn random_origin(seed: i64) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut x = (seed as u64) ^ (u64::from(n) << 40) ^ 0x9e37_79b9_7f4a_7c15;
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    format!("{:08x}", (x >> 32) as u32)
}
