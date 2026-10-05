//! Stride Prefetcher.
//!
//! A reference prediction table (Chen and Baer; gem5's `StridePrefetcher`)
//! indexed and tagged by the PC of the load that made the access, so each
//! static load learns its own stride whatever its magnitude. An access
//! without a PC (a store draining after commit, a page walk, a writeback)
//! does not train it. Prefetching starts once the same stride has repeated
//! enough to saturate the entry's confidence.

use super::Prefetcher;
use crate::common::VirtAddr;

/// Confidence at which an entry prefetches.
const MAX_CONFIDENCE: u8 = 3;

/// A load's entry in the reference prediction table.
#[derive(Clone, Copy, Debug)]
struct StrideEntry {
    /// The load this entry tracks.
    pc: VirtAddr,
    /// The address the load last accessed.
    last_addr: u64,
    /// The stride between its last two accesses that confidence is built on.
    stride: i64,
    /// Saturating confidence in `stride`.
    confidence: u8,
}

impl StrideEntry {
    const fn new(pc: VirtAddr, addr: u64) -> Self {
        Self { pc, last_addr: addr, stride: 0, confidence: 0 }
    }
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
    /// Number of strides to prefetch ahead.
    degree: usize,
}

impl StridePrefetcher {
    /// Creates a new Stride prefetcher.
    ///
    /// # Arguments
    ///
    /// * `line_bytes` - The size of a cache line in bytes.
    /// * `table_size` - Number of entries in the tracking table (must be power of 2).
    /// * `degree` - The number of strides to prefetch ahead.
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

    /// Instructions are at least 2-byte aligned, so bit 0 of a PC carries
    /// no information.
    const fn index(&self, pc: VirtAddr) -> usize {
        (pc.val() >> 1) as usize & self.table_mask
    }

    /// The lines `degree` strides ahead of `addr`.
    fn targets(&self, addr: u64, stride: i64) -> Vec<u64> {
        if stride == 0 {
            return Vec::new();
        }
        (1..=self.degree as i64)
            .map(|k| (addr as i64).wrapping_add(stride.wrapping_mul(k)) as u64)
            .map(|target| target & !(self.line_bytes - 1))
            .collect()
    }
}

impl Prefetcher for StridePrefetcher {
    /// Trains the entry of the load at `pc` on `addr` and, once its stride
    /// has repeated with full confidence, returns the lines `degree`
    /// strides ahead. A load whose entry another load holds takes it over.
    fn observe(&mut self, addr: u64, pc: Option<VirtAddr>, _hit: bool) -> Vec<u64> {
        let Some(pc) = pc else { return Vec::new() };
        let index = self.index(pc);
        let slot = &mut self.table[index];
        if slot.is_none_or(|entry| entry.pc != pc) {
            *slot = Some(StrideEntry::new(pc, addr));
            return Vec::new();
        }
        let Some(entry) = slot.as_mut() else { return Vec::new() };

        let stride = (addr as i64).wrapping_sub(entry.last_addr as i64);
        entry.last_addr = addr;
        if stride == entry.stride {
            if entry.confidence < MAX_CONFIDENCE {
                entry.confidence += 1;
                return Vec::new();
            }
            return self.targets(addr, stride);
        }
        if entry.confidence > 0 {
            entry.confidence -= 1;
        } else {
            entry.stride = stride;
        }
        Vec::new()
    }
}
