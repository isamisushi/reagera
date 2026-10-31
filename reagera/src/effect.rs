//! Effects: how an app talks to the outside world without leaving the log.
//!
//! `apply` and `resolve` are pure. When they need the outside world (a network
//! call, a payment, the clock, a random number) they emit a *request* through
//! [`Ctx::emit`]. The request is not written to the log: it is derived from the
//! record that emitted it, so every fold of the history finds the same request
//! with the same ID. The set of requests that have no answer yet is part of the
//! snapshot, so it is correct after a late event, a checkpoint, or an import.
//!
//! The runtime performs a pending request once, through [`App::perform`], and
//! records the answer as a [`Body::Reply`] record. That record is in the log.
//! When the history is replayed, the fold meets the reply and settles the
//! request without calling `perform` again. A payment made once stays made once.
//!
//! [`App::perform`]: crate::App::perform

use crate::{app::{App, Wire}, hash::fnv1a_128, record::Record};
use minbin::{BytesReader, BytesWriter, ToFromByteError, ToFromBytes};
use std::fmt;

/// What a record carries: a user event, or the outside world's answer to a request.
///
/// Wire layout: tag `u8` (0 event, 1 reply), then the fields in order.
#[derive(Clone, PartialEq, Eq)]
pub enum Body<E, F, O> {
    /// A user action.
    Event(E),
    /// The answer to a request. `effect` is the request's ID, `request` repeats
    /// the request so the record reads on its own, `outcome` is the answer.
    Reply {
        /// ID of the request this answers.
        effect: u128,
        /// The request, repeated.
        request: F,
        /// The answer.
        outcome: O,
    },
}

/// A record of app `A`.
pub type Rec<A> = Record<Body<<A as App>::Event, <A as App>::Effect, <A as App>::Outcome>>;

impl<E: Wire, F: Wire, O: Wire> Default for Body<E, F, O> {
    fn default() -> Self {
        Self::Event(E::default())
    }
}

impl<E: fmt::Debug, F: fmt::Debug, O: fmt::Debug> fmt::Debug for Body<E, F, O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Event(e) => e.fmt(f),
            Self::Reply { effect, request, outcome } => write!(f, "reply #{} {request:?} = {outcome:?}", short(*effect)),
        }
    }
}

impl<'a, E: Wire, F: Wire, O: Wire> ToFromBytes<'a> for Body<E, F, O> {
    const MAX_BYTES: usize = 1_048_576;

    fn to_bytes(&self, writer: &mut BytesWriter<'a>) -> Result<(), ToFromByteError> {
        match self {
            Self::Event(e) => {
                writer.write(&0u8)?;
                writer.write(e)
            }
            Self::Reply { effect, request, outcome } => {
                writer.write(&1u8)?;
                writer.write(effect)?;
                writer.write(request)?;
                writer.write(outcome)
            }
        }
    }

    fn from_bytes(buffer: &mut Self, reader: &mut BytesReader<'a>) -> Result<usize, ToFromByteError> {
        let mut tag = 0u8;
        reader.read_into(&mut tag)?;
        *buffer = match tag {
            0 => Self::Event(reader.read(E::default)?),
            1 => {
                let effect = reader.read(u128::default)?;
                let request = reader.read(F::default)?;
                let outcome = reader.read(O::default)?;
                Self::Reply { effect, request, outcome }
            }
            _ => return Err(ToFromByteError::InvalidValue),
        };
        Ok(reader.pos)
    }

    fn byte_count(&self) -> usize {
        1 + match self {
            Self::Event(e) => e.byte_count(),
            Self::Reply { effect, request, outcome } => effect.byte_count() + request.byte_count() + outcome.byte_count(),
        }
    }
}

/// A request that has no answer yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending<F> {
    /// Stable ID: a hash of the emitting record's ID and the request's index in it.
    pub id: u128,
    /// Origin of the record that emitted it. Only that origin performs it.
    pub origin: String,
    /// Millisecond of the record that emitted it.
    pub ms: i64,
    /// The request.
    pub request: F,
}

/// Callback that delivers an outcome to the runtime.
pub type Done<O> = Box<dyn FnOnce(O)>;

/// What `apply` and `resolve` may know besides the state, and the only way to emit a request.
pub struct Ctx<'a, A: App> {
    record: &'a Rec<A>,
    emitted: Vec<Pending<A::Effect>>,
}

impl<'a, A: App> Ctx<'a, A> {
    pub(crate) fn new(record: &'a Rec<A>) -> Self {
        Self { record, emitted: Vec::new() }
    }

    pub(crate) fn finish(self) -> Vec<Pending<A::Effect>> {
        self.emitted
    }

    /// The record being applied.
    pub fn record(&self) -> &Rec<A> {
        self.record
    }
    /// Millisecond of the record.
    pub fn ms(&self) -> i64 {
        self.record.ms()
    }
    /// Origin of the record.
    pub fn origin(&self) -> &str {
        self.record.origin()
    }
    /// Per-origin sequence number of the record.
    pub fn seq(&self) -> u32 {
        self.record.seq()
    }
    /// Content ID of the record. A good identity for anything the record creates.
    pub fn id(&self) -> u128 {
        self.record.id()
    }

    /// Ask the outside world for something. Returns the request's ID, which the
    /// reply will carry. The ID is the same on every machine and every fold.
    pub fn emit(&mut self, request: A::Effect) -> u128 {
        let id = effect_id(self.record.id(), self.emitted.len() as u32);
        self.emitted.push(Pending { id, origin: self.record.origin().to_string(), ms: self.record.ms(), request });
        id
    }
}

/// The ID of the `index`-th request emitted by the record with ID `record`.
pub fn effect_id(record: u128, index: u32) -> u128 {
    let mut bytes = [0u8; 20];
    bytes[..16].copy_from_slice(&record.to_be_bytes());
    bytes[16..].copy_from_slice(&index.to_be_bytes());
    fnv1a_128(&bytes)
}

/// First 8 hex digits of an ID, for logs and panels.
pub fn short(id: u128) -> String {
    format!("{:08x}", (id >> 96) as u32)
}

/// The effect or outcome type of an app that has no effects. Zero bytes on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nothing;

impl<'a> ToFromBytes<'a> for Nothing {
    const MAX_BYTES: usize = 0;
    fn to_bytes(&self, _: &mut BytesWriter<'a>) -> Result<(), ToFromByteError> {
        Ok(())
    }
    fn from_bytes(_: &mut Self, reader: &mut BytesReader<'a>) -> Result<usize, ToFromByteError> {
        Ok(reader.pos)
    }
    fn byte_count(&self) -> usize {
        0
    }
}
