//! The application's time type for ConTime.
//!
//! A tick is a wall-clock millisecond plus a per-origin sequence number. ConTime
//! orders events by time first, so two clicks in the same millisecond keep the
//! order they were made in, and a late event from another origin lands by its
//! own millisecond, behind nothing newer than itself.

use contime::{checkpoints::Timestamp, AdvanceTime};

/// A point in logical time: millisecond, then sequence within that millisecond.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick {
    /// Milliseconds since the Unix epoch, or any monotonic origin the app chooses.
    pub ms: i64,
    /// Sequence number within the millisecond.
    pub seq: u32,
}

impl Tick {
    /// The last tick of a millisecond. Querying at it includes every event in that millisecond.
    pub const fn end_of(ms: i64) -> Self {
        Self { ms, seq: u32::MAX }
    }
}

impl Timestamp for Tick {
    fn minimum() -> Self {
        Self { ms: i64::MIN, seq: 0 }
    }

    fn previous(&self) -> Option<Self> {
        if self.seq > 0 {
            Some(Self { ms: self.ms, seq: self.seq - 1 })
        } else {
            self.ms.checked_sub(1).map(|ms| Self { ms, seq: u32::MAX })
        }
    }
}

impl AdvanceTime for Tick {
    fn is_valid_retention(&self) -> bool {
        self.ms >= 0
    }

    fn saturating_sub(&self, retention: &Self) -> Self {
        Self { ms: self.ms.saturating_sub(retention.ms), seq: 0 }
    }
}
