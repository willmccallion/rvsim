//! Stride Prefetcher.
//!
//! A reference prediction table (Chen and Baer; gem5's `StridePrefetcher`)
//! indexed and tagged by the PC of the load that made the access, so each
//! static load learns its own stride whatever its magnitude. An access
//! without a PC (a store draining after commit, a page walk, a writeback)
//! does not train it. Prefetching starts once the same stride has repeated
//! enough to saturate the entry's confidence, and goes out a line at a time.

use super::Prefetcher;
use crate::common::VirtAddr;

/// Confidence at which a stream prefetches.
const MAX_CONFIDENCE: u8 = 3;

/// How one stream of accesses is learned: the address it last touched,
/// the stride confidence is built on, and a saturating confidence.
#[derive(Clone, Copy, Debug)]
pub struct StrideTracker {
    last_addr: u64,
    stride: i64,
    confidence: u8,
}

impl StrideTracker {
    /// A stream whose first access was to `addr`.
    pub const fn starting_at(addr: u64) -> Self {
        Self { last_addr: addr, stride: 0, confidence: 0 }
    }

    /// The stride being learned.
    pub const fn stride(&self) -> i64 {
        self.stride
    }

    /// Trains on the stream's next access; returns the stride once the
    /// same nonzero stride has repeated with full confidence.
    pub fn train(&mut self, addr: u64) -> Option<i64> {
        let stride = (addr as i64).wrapping_sub(self.last_addr as i64);
        self.last_addr = addr;
        if stride == self.stride {
            if self.confidence < MAX_CONFIDENCE {
                self.confidence += 1;
                return None;
            }
            return (stride != 0).then_some(stride);
        }
        if self.confidence > 0 {
            self.confidence -= 1;
        } else {
            self.stride = stride;
        }
        None
    }
}

/// The line `k` steps along `stride` from `addr`. Prefetches go out a line
/// at a time, so a stride shorter than a line steps a line.
pub const fn line_along(addr: u64, stride: i64, k: i64, line_bytes: u64) -> u64 {
    let line = line_bytes as i64;
    let step = if stride.abs() < line { line * stride.signum() } else { stride };
    (addr as i64).wrapping_add(step.wrapping_mul(k)) as u64 & !(line_bytes - 1)
}

/// The table slot of the load at `pc`: instructions are at least 2-byte
/// aligned, so bit 0 of a PC carries no information.
pub const fn pc_index(pc: VirtAddr, table_mask: usize) -> usize {
    (pc.val() >> 1) as usize & table_mask
}

/// A load's entry in the reference prediction table.
#[derive(Clone, Copy, Debug)]
struct StrideEntry {
    /// The load this entry tracks.
    pc: VirtAddr,
    /// Its stream.
    tracker: StrideTracker,
}

/// Stride Prefetcher state.
#[derive(Debug)]
pub struct StridePrefetcher {
    /// Reference prediction table, direct-mapped on the PC.
    table: Vec<Option<StrideEntry>>,
    /// Size of a cache line in bytes.
    line_bytes: u64,
    /// Mask used to index the table.
    table_mask: usize,
    /// Number of lines to prefetch ahead.
    degree: usize,
}

impl StridePrefetcher {
    /// Creates a new Stride prefetcher.
    ///
    /// # Arguments
    ///
    /// * `line_bytes` - The size of a cache line in bytes.
    /// * `table_size` - Number of entries in the tracking table (must be power of 2).
    /// * `degree` - The number of lines to prefetch ahead.
    pub fn new(line_bytes: usize, table_size: usize, degree: usize) -> Self {
        let safe_size =
            if table_size > 0 && table_size.is_power_of_two() { table_size } else { 64 };
        Self {
            table: vec![None; safe_size],
            line_bytes: line_bytes as u64,
            table_mask: safe_size - 1,
            degree: if degree == 0 { 1 } else { degree },
        }
    }
}

impl Prefetcher for StridePrefetcher {
    /// Trains the entry of the load at `pc` on `addr` and, once its stride
    /// has repeated with full confidence, returns the next `degree` lines
    /// along it. A load whose entry another load holds takes it over.
    fn observe(&mut self, addr: u64, pc: Option<VirtAddr>, _hit: bool) -> Vec<u64> {
        let Some(pc) = pc else { return Vec::new() };
        let slot = &mut self.table[pc_index(pc, self.table_mask)];
        if slot.is_none_or(|entry| entry.pc != pc) {
            *slot = Some(StrideEntry { pc, tracker: StrideTracker::starting_at(addr) });
            return Vec::new();
        }
        let Some(entry) = slot.as_mut() else { return Vec::new() };
        let Some(stride) = entry.tracker.train(addr) else { return Vec::new() };
        (1..=self.degree as i64).map(|k| line_along(addr, stride, k, self.line_bytes)).collect()
    }
}
