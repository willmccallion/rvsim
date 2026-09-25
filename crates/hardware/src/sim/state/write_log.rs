//! Per-line record of the most recent RAM write, kept only when more than
//! one hart can write memory.
//!
//! Functional data lives in RAM and a load reads it when its response
//! arrives, so a write by another hart that lands between that response
//! and the load's commit is invisible to the coherence protocol (caches
//! hold no data). The log makes that instant observable: every RAM write
//! records its sequence number and writer against its cache line, a load
//! response is stamped with the sequence current at that moment, and the
//! commit-time checks compare the two.

use crate::common::{HartId, PhysAddr};

/// Position in the global order of RAM writes. `WriteSeq::default()`
/// precedes every write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct WriteSeq(u64);

/// Who performed a RAM write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writer {
    /// A hart's committed store, SC, AMO or vector store.
    Hart(HartId),
    /// The loader or a host-side probe; counts as "another hart" for every
    /// hart.
    External,
}

const WRITER_BITS: u32 = 8;
const WRITER_MASK: u64 = (1 << WRITER_BITS) - 1;
const EXTERNAL_TAG: u64 = 0;

/// Most recent write per cache line.
#[derive(Debug)]
pub struct WriteLog {
    ram_base: u64,
    line_shift: u32,
    next: u64,
    /// `seq << WRITER_BITS | writer_tag` per line; zero means never written.
    last: Vec<u64>,
}

impl WriteLog {
    /// A log covering `ram_size` bytes of RAM at `ram_base`, tracking
    /// writes at `line_bytes` granularity.
    ///
    /// # Panics
    ///
    /// Panics if `line_bytes` is not a power of two or `hart_count` exceeds
    /// the writer tag space.
    #[must_use]
    pub fn new(ram_base: u64, ram_size: u64, line_bytes: u64, hart_count: usize) -> Self {
        assert!(line_bytes.is_power_of_two(), "write log line size must be a power of two");
        assert!(hart_count < WRITER_MASK as usize, "write log supports at most 254 harts");
        let line_shift = line_bytes.trailing_zeros();
        let lines = usize::try_from(ram_size.div_ceil(line_bytes)).unwrap_or(usize::MAX);
        Self { ram_base, line_shift, next: 1, last: vec![0; lines] }
    }

    /// Granularity of the log in bytes.
    #[must_use]
    pub const fn line_bytes(&self) -> u64 {
        1 << self.line_shift
    }

    /// Sequence of the most recent write; a load stamped with this value
    /// has seen every write up to and including it.
    #[must_use]
    pub const fn now(&self) -> WriteSeq {
        WriteSeq(self.next - 1)
    }

    fn line_index(&self, paddr: PhysAddr) -> Option<usize> {
        let offset = paddr.val().checked_sub(self.ram_base)?;
        let index = usize::try_from(offset >> self.line_shift).ok()?;
        (index < self.last.len()).then_some(index)
    }

    const fn writer_tag(writer: Writer) -> u64 {
        match writer {
            Writer::Hart(hart) => hart.val() as u64 + 1,
            Writer::External => EXTERNAL_TAG,
        }
    }

    /// Records a write to the line containing `paddr`. Addresses outside
    /// RAM are ignored.
    pub fn record(&mut self, paddr: PhysAddr, writer: Writer) {
        let Some(index) = self.line_index(paddr) else { return };
        let seq = self.next;
        self.next += 1;
        self.last[index] = (seq << WRITER_BITS) | Self::writer_tag(writer);
    }

    /// True when a writer other than `reader` has written the line
    /// containing `paddr` after `since`.
    #[must_use]
    pub fn written_by_other_since(&self, paddr: PhysAddr, reader: HartId, since: WriteSeq) -> bool {
        let Some(index) = self.line_index(paddr) else { return false };
        let entry = self.last[index];
        if entry == 0 {
            return false;
        }
        let seq = entry >> WRITER_BITS;
        let tag = entry & WRITER_MASK;
        seq > since.0 && tag != Self::writer_tag(Writer::Hart(reader))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H0: HartId = HartId::new(0);
    const H1: HartId = HartId::new(1);

    fn log() -> WriteLog {
        WriteLog::new(0x8000_0000, 0x1_0000, 64, 2)
    }

    #[test]
    fn a_write_after_the_stamp_by_another_hart_is_seen() {
        let mut log = log();
        let stamp = log.now();
        log.record(PhysAddr::new(0x8000_0008), Writer::Hart(H1));
        assert!(log.written_by_other_since(PhysAddr::new(0x8000_0000), H0, stamp));
        assert!(log.written_by_other_since(PhysAddr::new(0x8000_003F), H0, stamp), "same line");
        assert!(!log.written_by_other_since(PhysAddr::new(0x8000_0040), H0, stamp), "next line");
    }

    #[test]
    fn a_write_before_the_stamp_is_not_seen() {
        let mut log = log();
        log.record(PhysAddr::new(0x8000_0000), Writer::Hart(H1));
        let stamp = log.now();
        assert!(!log.written_by_other_since(PhysAddr::new(0x8000_0000), H0, stamp));
    }

    #[test]
    fn the_readers_own_writes_do_not_count() {
        let mut log = log();
        let stamp = log.now();
        log.record(PhysAddr::new(0x8000_0000), Writer::Hart(H0));
        assert!(!log.written_by_other_since(PhysAddr::new(0x8000_0000), H0, stamp));
        assert!(log.written_by_other_since(PhysAddr::new(0x8000_0000), H1, stamp));
    }

    #[test]
    fn external_writes_count_for_every_hart() {
        let mut log = log();
        let stamp = log.now();
        log.record(PhysAddr::new(0x8000_0000), Writer::External);
        assert!(log.written_by_other_since(PhysAddr::new(0x8000_0000), H0, stamp));
        assert!(log.written_by_other_since(PhysAddr::new(0x8000_0000), H1, stamp));
    }

    #[test]
    fn the_latest_writer_wins() {
        let mut log = log();
        let stamp = log.now();
        log.record(PhysAddr::new(0x8000_0000), Writer::Hart(H1));
        log.record(PhysAddr::new(0x8000_0000), Writer::Hart(H0));
        assert!(!log.written_by_other_since(PhysAddr::new(0x8000_0000), H0, stamp), "hart 0's own write was last");
    }

    #[test]
    fn addresses_outside_ram_are_ignored() {
        let mut log = log();
        let stamp = log.now();
        log.record(PhysAddr::new(0x1000_0000), Writer::Hart(H1));
        assert_eq!(log.now(), stamp);
        assert!(!log.written_by_other_since(PhysAddr::new(0x1000_0000), H0, stamp));
    }
}
