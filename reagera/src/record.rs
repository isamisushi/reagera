//! One event in the log.

use crate::{app::Wire, hash::fnv1a_128, tick::Tick};
use contime::{checkpoints::Event, Input};
use minbin::{BytesReader, BytesWriter, ToFromByteError, ToFromBytes};

/// The single ConTime history every record belongs to. A page has one state tree.
pub const ROOT_HISTORY: u128 = 1;

/// A timestamped, attributed event with a stable content ID.
///
/// Wire layout, in order: `ms: i64`, `origin: String`, `seq: u32`, `body: E`.
/// The ID is not stored; it is the FNV-1a-128 hash of those bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record<E> {
    ms: i64,
    origin: String,
    seq: u32,
    body: E,
    id: u128,
}

impl<E: Wire> Record<E> {
    /// Build a record and compute its ID.
    pub fn new(ms: i64, origin: impl Into<String>, seq: u32, body: E) -> Self {
        let mut r = Self { ms, origin: origin.into(), seq, body, id: 0 };
        r.id = r.compute_id();
        r
    }

    fn compute_id(&self) -> u128 {
        let bytes = minbin::to_bytes(self).expect("record fits the wire format");
        fnv1a_128(&bytes)
    }

    /// Millisecond the event happened at.
    pub fn ms(&self) -> i64 {
        self.ms
    }
    /// Who produced the event.
    pub fn origin(&self) -> &str {
        &self.origin
    }
    /// Per-origin sequence number.
    pub fn seq(&self) -> u32 {
        self.seq
    }
    /// The application event.
    pub fn body(&self) -> &E {
        &self.body
    }
    /// Content ID.
    pub fn id(&self) -> u128 {
        self.id
    }
    /// The ConTime timestamp.
    pub fn tick(&self) -> Tick {
        Tick { ms: self.ms, seq: self.seq }
    }
}

impl<E: Wire> Default for Record<E> {
    fn default() -> Self {
        Self { ms: 0, origin: String::new(), seq: 0, body: E::default(), id: 0 }
    }
}

impl<'a, E: Wire> ToFromBytes<'a> for Record<E> {
    const MAX_BYTES: usize = 1_048_576;

    fn to_bytes(&self, writer: &mut BytesWriter<'a>) -> Result<(), ToFromByteError> {
        writer.write(&self.ms)?;
        writer.write(&self.origin)?;
        writer.write(&self.seq)?;
        writer.write(&self.body)
    }

    fn from_bytes(buffer: &mut Self, reader: &mut BytesReader<'a>) -> Result<usize, ToFromByteError> {
        reader.read_into(&mut buffer.ms)?;
        reader.read_into(&mut buffer.origin)?;
        reader.read_into(&mut buffer.seq)?;
        reader.read_into(&mut buffer.body)?;
        buffer.id = buffer.compute_id();
        Ok(reader.pos)
    }

    fn byte_count(&self) -> usize {
        self.ms.byte_count() + self.origin.byte_count() + self.seq.byte_count() + self.body.byte_count()
    }
}

impl<E: Wire> Event for Record<E> {
    type Time = Tick;
    fn time(&self) -> Tick {
        self.tick()
    }
}

impl<E: Wire> Input for Record<E> {
    fn event_id(&self) -> u128 {
        self.id
    }
    fn snapshot_ids(&self, emit: &mut impl FnMut(u128)) {
        emit(ROOT_HISTORY);
    }
}
