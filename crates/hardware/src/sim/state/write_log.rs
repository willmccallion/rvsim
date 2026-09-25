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
//!
//! Each line remembers its latest write and the latest write by a
//! different writer, which is enough to answer "has anyone but me written
//! this line since?" exactly: the reader's own later write to the line
//! must not hide another hart's earlier one.

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

/// A write as `seq << WRITER_BITS | writer_tag`; zero means none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Write(u64);

impl Write {
    const NONE: Self = Self(0);

    const fn new(seq: u64, writer: Writer) -> Self {
        Self((seq << WRITER_BITS) | WriteLog::writer_tag(writer))
    }

    const fn seq(self) -> u64 {
        self.0 >> WRITER_BITS
    }

    const fn tag(self) -> u64 {
        self.0 & WRITER_MASK
    }
}

/// The latest write to a line, and the latest one by a different writer.
#[derive(Clone, Copy, Debug, Default)]
struct LineWrites {
    latest: Write,
    latest_by_other: Write,
}

/// Most recent writes per cache line.
#[derive(Debug)]
pub struct WriteLog {
    ram_base: u64,
    line_shift: u32,
    next: u64,
    lines: Vec<LineWrites>,
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
        Self { ram_base, line_shift, next: 1, lines: vec![LineWrites::default(); lines] }
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
        (index < self.lines.len()).then_some(index)
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
        let write = Write::new(seq, writer);
        let line = &mut self.lines[index];
        if line.latest != Write::NONE && line.latest.tag() != write.tag() {
            line.latest_by_other = line.latest;
        }
        line.latest = write;
    }

    /// True when a writer other than `reader` has written the line
    /// containing `paddr` after `since`.
    #[must_use]
    pub fn written_by_other_since(&self, paddr: PhysAddr, reader: HartId, since: WriteSeq) -> bool {
        let Some(index) = self.line_index(paddr) else { return false };
        let line = self.lines[index];
        let reader_tag = Self::writer_tag(Writer::Hart(reader));
        let by_other = if line.latest.tag() == reader_tag { line.latest_by_other } else { line.latest };
        by_other != Write::NONE && by_other.seq() > since.0
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
    fn the_readers_own_later_write_does_not_hide_anothers() {
        let mut log = log();
        let stamp = log.now();
        log.record(PhysAddr::new(0x8000_0000), Writer::Hart(H1));
        log.record(PhysAddr::new(0x8000_0008), Writer::Hart(H0));
        assert!(log.written_by_other_since(PhysAddr::new(0x8000_0000), H0, stamp), "hart 1 wrote after the stamp");
        assert!(log.written_by_other_since(PhysAddr::new(0x8000_0000), H1, stamp), "hart 0 wrote after the stamp");
    }

    #[test]
    fn only_writes_after_the_stamp_count_whoever_wrote_last() {
        let mut log = log();
        log.record(PhysAddr::new(0x8000_0000), Writer::Hart(H1));
        let stamp = log.now();
        log.record(PhysAddr::new(0x8000_0000), Writer::Hart(H0));
        log.record(PhysAddr::new(0x8000_0000), Writer::Hart(H0));
        assert!(!log.written_by_other_since(PhysAddr::new(0x8000_0000), H0, stamp), "hart 1's write predates the stamp");
        assert!(log.written_by_other_since(PhysAddr::new(0x8000_0000), H1, stamp));
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
