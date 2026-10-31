//! A whole history as bytes.
//!
//! Layout: magic `u32` "RGRA", version `u8`, app name, `live: bool`,
//! `head_ms: i64`, then the records as a `Vec`. Nothing else. Record IDs are
//! recomputed on import, so a session cannot carry a record whose ID disagrees
//! with its content.

use crate::{app::Wire, record::Record};
use minbin::{BytesReader, BytesWriter, ToFromByteError, ToFromBytes};

const MAGIC: u32 = 0x5247_5241;
const VERSION: u8 = 2;

/// The serializable form of a store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session<E> {
    /// `App::NAME` of the app that produced it.
    pub name: String,
    /// Whether the head followed the newest event.
    pub live: bool,
    /// Head millisecond when not live.
    pub head_ms: i64,
    /// Every retained record in canonical order.
    pub records: Vec<Record<E>>,
}

impl<'a, E: Wire> ToFromBytes<'a> for Session<E> {
    const MAX_BYTES: usize = 64 * 1_048_576;

    fn to_bytes(&self, writer: &mut BytesWriter<'a>) -> Result<(), ToFromByteError> {
        writer.write(&MAGIC)?;
        writer.write(&VERSION)?;
        writer.write(&self.name)?;
        writer.write(&self.live)?;
        writer.write(&self.head_ms)?;
        writer.write(&self.records)
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
        reader.read_into(&mut buffer.records)?;
        Ok(reader.pos)
    }

    fn byte_count(&self) -> usize {
        MAGIC.byte_count() + VERSION.byte_count() + self.name.byte_count() + self.live.byte_count() + self.head_ms.byte_count() + self.records.byte_count()
    }
}
