//! A whole history as bytes.
//!
//! Layout: magic `u32` "RGRA", version `u8`, app name, `live: bool`,
//! `head_ms: i64`, a `u32` record count, then the records. Nothing else. The
//! records are written one by one rather than as a minbin `Vec`, because a
//! `Vec` is capped at 1 MiB and a session may be 64 MiB. Record IDs are
//! recomputed on import, so a session cannot carry a record whose ID disagrees
//! with its content. Pending requests are not stored: they are derived from
//! the records when the history is folded.

use crate::{app::Wire, record::Record};
use minbin::{BytesReader, BytesWriter, ToFromByteError, ToFromBytes};

const MAGIC: u32 = 0x5247_5241;
const VERSION: u8 = 3;

/// The serializable form of a store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session<B> {
    /// `App::NAME` of the app that produced it.
    pub name: String,
    /// Whether the head followed the newest event.
    pub live: bool,
    /// Head millisecond when not live.
    pub head_ms: i64,
    /// Every retained record in canonical order.
    pub records: Vec<Record<B>>,
}

impl<'a, B: Wire> ToFromBytes<'a> for Session<B> {
    const MAX_BYTES: usize = 64 * 1_048_576;

    fn to_bytes(&self, writer: &mut BytesWriter<'a>) -> Result<(), ToFromByteError> {
        writer.write(&MAGIC)?;
        writer.write(&VERSION)?;
        writer.write(&self.name)?;
        writer.write(&self.live)?;
        writer.write(&self.head_ms)?;
        let count = u32::try_from(self.records.len()).map_err(|_| ToFromByteError::InvalidValue)?;
        writer.write(&count)?;
        for record in &self.records {
            writer.write(record)?;
        }
        Ok(())
    }

    fn from_bytes(buffer: &mut Self, reader: &mut BytesReader<'a>) -> Result<usize, ToFromByteError> {
        let mut magic = 0u32;
        reader.read_into(&mut magic)?;
        let mut version = 0u8;
        reader.read_into(&mut version)?;
        if magic != MAGIC || version != VERSION {
            return Err(ToFromByteError::InvalidValue);
        }
        reader.read_into(&mut buffer.name)?;
        reader.read_into(&mut buffer.live)?;
        reader.read_into(&mut buffer.head_ms)?;
        let mut count = 0u32;
        reader.read_into(&mut count)?;
        buffer.records.clear();
        for _ in 0..count {
            buffer.records.push(reader.read(Record::default)?);
        }
        Ok(reader.pos)
    }

    fn byte_count(&self) -> usize {
        MAGIC.byte_count() + VERSION.byte_count() + self.name.byte_count() + self.live.byte_count() + self.head_ms.byte_count() + 4 + self.records.iter().map(Record::byte_count).sum::<usize>()
    }
}
