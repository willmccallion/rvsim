//! Miss Status Holding Registers (MSHRs) for non-blocking cache access.
//!
//! MSHRs track outstanding cache misses and allow multiple misses to be
//! in-flight simultaneously, enabling Memory-Level Parallelism (MLP).
//! When a second miss arrives for the same cache line, it coalesces with
//! the existing MSHR entry instead of allocating a new one.

use crate::core::pipeline::latches::Mem1Mem2Entry;
use crate::core::pipeline::rob::RobTag;

/// State of an MSHR entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MshrState {
    /// Waiting for data from next cache level / DRAM.
    Pending,
    /// Data has arrived; ready to install line and wake waiters.
    Complete,
}

/// A single waiter attached to an MSHR.
#[derive(Clone, Debug)]
pub struct MshrWaiter {
    /// ROB tag of the waiting instruction.
    pub rob_tag: RobTag,
    /// Full pipeline entry parked until the line arrives.
    /// None for fire-and-forget requests (e.g. store write-allocate).
    pub parked_entry: Option<Mem1Mem2Entry>,
}

/// A single MSHR entry tracking one outstanding cache line fetch.
#[derive(Clone, Debug)]
pub struct MshrEntry {
    /// Cache-line-aligned physical address.
    pub line_addr: u64,
    /// Current state.
    pub state: MshrState,
    /// Cycle at which the miss data will be available.
    pub complete_cycle: u64,
    /// Instructions waiting on this line.
    pub waiters: Vec<MshrWaiter>,
    /// Whether this slot is valid.
    pub valid: bool,
    /// Whether the original access was a write (for `install_line` dirty bit).
    pub is_write: bool,
}

/// Result of attempting an MSHR-aware cache access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheResponse {
    /// L1D hit. No MSHR involvement.
    Hit,
    /// Miss, new MSHR allocated.
    MshrAllocated {
        /// Cycle at which the miss fill will complete.
        complete_cycle: u64,
    },
    /// Miss, but same line already has an outstanding MSHR (coalesced).
    MshrCoalesced {
        /// Cycle at which the existing fill will complete.
        complete_cycle: u64,
    },
    /// Miss, all MSHRs are full. Caller must stall this access.
    MshrFull,
}

/// File of MSHRs for a single cache level.
#[derive(Debug)]
pub struct MshrFile {
    entries: Vec<MshrEntry>,
    cap: usize,
    count: usize,
    line_bytes: usize,
}

impl MshrFile {
    /// Create a new MSHR file with the given capacity and cache line size.
    pub fn new(capacity: usize, line_bytes: usize) -> Self {
        let safe_line = if line_bytes == 0 { 64 } else { line_bytes };
        let entries = vec![
            MshrEntry {
                line_addr: 0,
                state: MshrState::Pending,
                complete_cycle: 0,
                waiters: Vec::new(),
                valid: false,
                is_write: false,
            };
            capacity
        ];
        Self { entries, cap: capacity, count: 0, line_bytes: safe_line }
    }

    /// Align an address to the cache line boundary.
    #[inline]
    const fn line_align(&self, addr: u64) -> u64 {
        addr & !(self.line_bytes as u64 - 1)
    }

    /// Find index of an existing MSHR for this cache line.
    fn find_line(&self, line_addr: u64) -> Option<usize> {
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.valid && entry.line_addr == line_addr {
                return Some(i);
            }
        }
        None
    }

    /// Attempt to allocate or coalesce an MSHR for the given address.
    ///
    /// `miss_latency` is the total penalty from the next-level cache/DRAM.
    /// `current_cycle` is the current simulation cycle.
    pub fn request(
        &mut self,
        addr: u64,
        is_write: bool,
        miss_latency: u64,
        current_cycle: u64,
        waiter: MshrWaiter,
    ) -> CacheResponse {
        let line_addr = self.line_align(addr);

        // Check for existing MSHR (coalesce)
        if let Some(idx) = self.find_line(line_addr) {
            let complete_cycle = self.entries[idx].complete_cycle;
            self.entries[idx].waiters.push(waiter);
            if is_write {
                self.entries[idx].is_write = true;
            }
            return CacheResponse::MshrCoalesced { complete_cycle };
        }

        // Allocate new MSHR
        if self.count >= self.cap {
            return CacheResponse::MshrFull;
        }

        for entry in &mut self.entries {
            if !entry.valid {
                let complete_cycle = current_cycle + miss_latency;
                *entry = MshrEntry {
                    line_addr,
                    state: MshrState::Pending,
                    complete_cycle,
                    waiters: vec![waiter],
                    valid: true,
                    is_write,
                };
                self.count += 1;
                return CacheResponse::MshrAllocated { complete_cycle };
            }
        }

        CacheResponse::MshrFull
    }

    /// Check for completed MSHRs and return them.
    ///
    /// Completed entries are freed. The caller is responsible for
    /// installing the cache line and resuming parked loads.
    pub fn drain_completions(&mut self, current_cycle: u64) -> Vec<MshrEntry> {
        let mut completed = Vec::new();
        for entry in &mut self.entries {
            if entry.valid && entry.complete_cycle <= current_cycle {
                let mut taken = MshrEntry {
                    line_addr: 0,
                    state: MshrState::Complete,
                    complete_cycle: 0,
                    waiters: Vec::new(),
                    valid: false,
                    is_write: false,
                };
                std::mem::swap(entry, &mut taken);
                taken.state = MshrState::Complete;
                self.count -= 1;
                completed.push(taken);
            }
        }
        completed
    }

    /// Remove waiters with `rob_tag > keep_tag` (misprediction recovery).
    ///
    /// Does NOT free the MSHR entry even if all waiters are removed — the
    /// cache line fetch is already in progress and installing it is always
    /// beneficial for future accesses.
    /// Flush waiters with `from_tag` or newer. Keeps only strictly older waiters.
    pub fn flush_from(&mut self, from_tag: RobTag) {
        for entry in &mut self.entries {
            if !entry.valid {
                continue;
            }
            entry.waiters.retain(|w| w.rob_tag.is_older_than(from_tag));
        }
    }

    /// Flush waiters newer than `keep_tag`. Keeps waiters at or before `keep_tag`.
    pub fn flush_after(&mut self, keep_tag: RobTag) {
        for entry in &mut self.entries {
            if !entry.valid {
                continue;
            }
            entry.waiters.retain(|w| w.rob_tag.is_older_or_eq(keep_tag));
        }
    }

    /// Flush all MSHRs.
    pub fn flush(&mut self) {
        for entry in &mut self.entries {
            entry.valid = false;
            entry.waiters.clear();
        }
        self.count = 0;
    }

    /// Number of active MSHRs.
    #[inline]
    pub const fn active_count(&self) -> usize {
        self.count
    }

    /// Whether all MSHRs are occupied.
    #[inline]
    pub const fn is_full(&self) -> bool {
        self.count >= self.cap
    }

    /// Returns the configured capacity (0 = blocking cache, no MSHRs).
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.cap
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::core::pipeline::rob::RobTag;

    fn make_waiter(tag: u32) -> MshrWaiter {
        MshrWaiter { rob_tag: RobTag(tag), parked_entry: None }
    }

    #[test]
    fn test_allocate_and_complete() {
        let mut mf = MshrFile::new(4, 64);
        assert_eq!(mf.active_count(), 0);

        let resp = mf.request(0x1000, false, 100, 10, make_waiter(1));
        assert!(matches!(resp, CacheResponse::MshrAllocated { complete_cycle: 110 }));
        assert_eq!(mf.active_count(), 1);

        // Not yet complete at cycle 50
        let completed = mf.drain_completions(50);
        assert!(completed.is_empty());
        assert_eq!(mf.active_count(), 1);

        // Complete at cycle 110
        let completed = mf.drain_completions(110);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].line_addr, 0x1000);
        assert_eq!(completed[0].waiters.len(), 1);
        assert_eq!(mf.active_count(), 0);
    }

    #[test]
    fn test_coalesce() {
        let mut mf = MshrFile::new(4, 64);

        // First miss to line 0x1000
        let resp = mf.request(0x1000, false, 100, 10, make_waiter(1));
        assert!(matches!(resp, CacheResponse::MshrAllocated { .. }));

        // Second miss to same line (different offset within line)
        let resp = mf.request(0x1020, false, 100, 15, make_waiter(2));
        assert!(matches!(resp, CacheResponse::MshrCoalesced { complete_cycle: 110 }));
        assert_eq!(mf.active_count(), 1); // Still just one MSHR

        // Complete — both waiters should be returned
        let completed = mf.drain_completions(110);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].waiters.len(), 2);
    }

    #[test]
    fn test_mshr_full() {
        let mut mf = MshrFile::new(2, 64);

        mf.request(0x1000, false, 100, 10, make_waiter(1));
        mf.request(0x2000, false, 100, 10, make_waiter(2));
        assert!(mf.is_full());

        // Third miss to different line — should be full
        let resp = mf.request(0x3000, false, 100, 10, make_waiter(3));
        assert_eq!(resp, CacheResponse::MshrFull);
    }

    #[test]
    fn test_flush_after() {
        let mut mf = MshrFile::new(4, 64);

        mf.request(0x1000, false, 100, 10, make_waiter(1));
        mf.request(0x1000, false, 100, 15, make_waiter(5)); // coalesce
        mf.request(0x2000, false, 100, 10, make_waiter(3));

        // Keep only tags <= 2
        mf.flush_after(RobTag(2));

        // MSHR for 0x1000 still has waiter tag=1, lost tag=5
        // MSHR for 0x2000 lost its only waiter tag=3 but entry stays
        let completed = mf.drain_completions(200);
        assert_eq!(completed.len(), 2);
        let mshr_1000 = completed.iter().find(|e| e.line_addr == 0x1000).unwrap();
        assert_eq!(mshr_1000.waiters.len(), 1);
        assert_eq!(mshr_1000.waiters[0].rob_tag.0, 1);
        let mshr_2000 = completed.iter().find(|e| e.line_addr == 0x2000).unwrap();
        assert_eq!(mshr_2000.waiters.len(), 0);
    }

    #[test]
    fn test_flush_all() {
        let mut mf = MshrFile::new(4, 64);
        mf.request(0x1000, false, 100, 10, make_waiter(1));
        mf.request(0x2000, false, 100, 10, make_waiter(2));

        mf.flush();
        assert_eq!(mf.active_count(), 0);
        assert!(!mf.is_full());
    }

    #[test]
    fn test_independent_misses() {
        let mut mf = MshrFile::new(4, 64);

        // Two independent misses at same time
        let r1 = mf.request(0x1000, false, 100, 10, make_waiter(1));
        let r2 = mf.request(0x2000, false, 100, 10, make_waiter(2));
        assert!(matches!(r1, CacheResponse::MshrAllocated { complete_cycle: 110 }));
        assert!(matches!(r2, CacheResponse::MshrAllocated { complete_cycle: 110 }));
        assert_eq!(mf.active_count(), 2);

        // Both complete at same cycle
        let completed = mf.drain_completions(110);
        assert_eq!(completed.len(), 2);
        assert_eq!(mf.active_count(), 0);
    }

    #[test]
    fn test_zero_capacity() {
        let mut mf = MshrFile::new(0, 64);
        assert_eq!(mf.capacity(), 0);
        let resp = mf.request(0x1000, false, 100, 10, make_waiter(1));
        assert_eq!(resp, CacheResponse::MshrFull);
    }

    #[test]
    fn test_write_allocate_fire_and_forget() {
        let mut mf = MshrFile::new(4, 64);

        // Store miss: allocate with is_write=true, waiter has no parked entry
        let waiter = MshrWaiter { rob_tag: RobTag(1), parked_entry: None };
        let resp = mf.request(0x1000, true, 80, 10, waiter);
        assert!(matches!(resp, CacheResponse::MshrAllocated { complete_cycle: 90 }));
        assert_eq!(mf.active_count(), 1);

        // Completion returns the entry with is_write=true and empty parked_entry
        let completed = mf.drain_completions(90);
        assert_eq!(completed.len(), 1);
        assert!(completed[0].is_write);
        assert!(completed[0].waiters[0].parked_entry.is_none());
    }

    #[test]
    fn test_reuse_after_completion() {
        let mut mf = MshrFile::new(2, 64);

        // Fill both MSHRs
        mf.request(0x1000, false, 100, 10, make_waiter(1));
        mf.request(0x2000, false, 100, 10, make_waiter(2));
        assert!(mf.is_full());

        // Complete one
        let completed = mf.drain_completions(110);
        assert_eq!(completed.len(), 2);
        assert_eq!(mf.active_count(), 0);

        // Slot should be reusable
        let resp = mf.request(0x3000, false, 50, 200, make_waiter(3));
        assert!(matches!(resp, CacheResponse::MshrAllocated { complete_cycle: 250 }));
        assert_eq!(mf.active_count(), 1);
    }

    #[test]
    fn test_coalesce_store_upgrades_write_bit() {
        let mut mf = MshrFile::new(4, 64);

        // Load miss first (is_write=false)
        mf.request(0x1000, false, 100, 10, make_waiter(1));

        // Store to same line coalesces and sets is_write=true
        mf.request(0x1008, true, 100, 15, make_waiter(2));

        let completed = mf.drain_completions(110);
        assert_eq!(completed.len(), 1);
        assert!(completed[0].is_write); // upgraded to write
        assert_eq!(completed[0].waiters.len(), 2);
    }

    #[test]
    fn test_flush_after_preserves_entry_for_line_install() {
        let mut mf = MshrFile::new(4, 64);

        // Single waiter that will be flushed
        mf.request(0x1000, false, 100, 10, make_waiter(5));

        // Flush everything after tag 2 — waiter tag=5 is removed
        mf.flush_after(RobTag(2));

        // MSHR entry still valid (active_count unchanged) — line fetch in progress
        assert_eq!(mf.active_count(), 1);

        // Completion still fires, just with no waiters
        let completed = mf.drain_completions(110);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].waiters.len(), 0);
        assert_eq!(completed[0].line_addr, 0x1000);
    }
}
