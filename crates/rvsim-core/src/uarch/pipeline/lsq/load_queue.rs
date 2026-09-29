//! Load Queue for in-flight load tracking and memory ordering violation detection.
//!
//! Tracks pending loads and detects memory ordering violations when a store
//! resolves its address and overlaps with a younger load that has already
//! executed with potentially stale data.
//!
//! ## Structure
//!
//! Unlike the scalar [`StoreBuffer`](super::store_buffer::StoreBuffer), the
//! load queue is a **set of slots**, not a circular FIFO. Vector load element
//! micro-ops complete out of program order — under wave-based issue, an
//! element near the tail can release its slot before an element at the head
//! finishes — and a circular FIFO with lazy invalidation deadlocks under
//! that pattern (middle invalidations don't free the slot until the head
//! catches up). This implementation reuses any invalidated slot for the
//! next allocation; ROB ordering is recovered from `rob_tag` on each entry.

use crate::common::{HartId, PhysAddr, VirtAddr};
use crate::isa::op::MemWidth;
use crate::sim::memory::write_log::{WriteLog, WriteSeq};
use crate::uarch::pipeline::latches::MicroOpIdx;
use crate::uarch::pipeline::rob::RobTag;

/// Lifecycle state of a load queue entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LoadState {
    /// Allocated but address not yet translated.
    #[default]
    Pending,
    /// Address translated (paddr filled).
    Translated,
    /// Data read from memory (load complete).
    Executed,
}

/// A single entry in the load queue.
#[derive(Clone, Debug, Default)]
pub struct LoadQueueEntry {
    /// ROB tag of the load instruction.
    pub rob_tag: RobTag,
    /// Virtual address of the load.
    pub vaddr: VirtAddr,
    /// Physical address (filled after translation).
    pub paddr: Option<PhysAddr>,
    /// Data read from memory.
    pub data: u64,
    /// Bytes the load reads.
    pub bytes: usize,
    /// Current lifecycle state.
    pub state: LoadState,
    /// Whether this slot is occupied.
    pub valid: bool,
    /// The vector load micro-op this entry tracks (`None` for a scalar load).
    pub micro_op: Option<MicroOpIdx>,
    /// Write-log position when the value was read from RAM (`None` when
    /// forwarded from the store buffer or in a single-hart system).
    pub observed: Option<WriteSeq>,
}

/// Load queue — bounded set of in-flight loads.
#[derive(Debug)]
pub struct LoadQueue {
    entries: Vec<LoadQueueEntry>,
    /// Cached count of valid entries.
    valid_count: usize,
}

impl LoadQueue {
    /// Creates a new load queue with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let mut entries = Vec::with_capacity(capacity);
        entries.resize_with(capacity, LoadQueueEntry::default);
        Self { entries, valid_count: 0 }
    }

    /// Returns the capacity.
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Returns the number of valid entries.
    #[inline]
    pub const fn len(&self) -> usize {
        self.valid_count
    }

    /// Returns true if the load queue is empty.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.valid_count == 0
    }

    /// Returns true if the load queue is full.
    #[inline]
    pub const fn is_full(&self) -> bool {
        self.valid_count == self.entries.len()
    }

    /// Returns the number of free slots.
    #[inline]
    pub const fn free_slots(&self) -> usize {
        self.entries.len() - self.valid_count
    }

    /// Allocates a slot for a new load of `bytes` bytes. Reuses any
    /// invalidated slot. Returns false if every slot is currently valid.
    ///
    /// `micro_op` is `None` for scalar loads and names a vector load's
    /// micro-op otherwise.
    pub fn allocate(
        &mut self,
        rob_tag: RobTag,
        bytes: usize,
        micro_op: Option<MicroOpIdx>,
    ) -> bool {
        let Some(slot) = self.entries.iter_mut().find(|e| !e.valid) else {
            return false;
        };
        *slot = LoadQueueEntry {
            rob_tag,
            vaddr: VirtAddr::new(0),
            paddr: None,
            data: 0,
            bytes,
            state: LoadState::Pending,
            valid: true,
            micro_op,
            observed: None,
        };
        self.valid_count += 1;
        true
    }

    /// Fills the translated address for a load after Memory1.
    pub fn fill_address(
        &mut self,
        rob_tag: RobTag,
        micro_op: Option<MicroOpIdx>,
        vaddr: VirtAddr,
        paddr: PhysAddr,
    ) {
        if let Some(entry) = self.find_mut(rob_tag, micro_op) {
            entry.vaddr = vaddr;
            entry.paddr = Some(paddr);
            entry.state = LoadState::Translated;
        }
    }

    /// Fills the loaded data for a load after Memory2, with the write-log
    /// position at which it was read.
    pub fn fill_data(
        &mut self,
        rob_tag: RobTag,
        micro_op: Option<MicroOpIdx>,
        data: u64,
        observed: Option<WriteSeq>,
    ) {
        if let Some(entry) = self.find_mut(rob_tag, micro_op) {
            entry.data = data;
            entry.state = LoadState::Executed;
            entry.observed = observed;
        }
    }

    /// Checks for memory ordering violations when a store resolves its address.
    ///
    /// Scans for younger loads (`rob_tag` > `store_rob_tag`) that have already
    /// translated their address (and therefore either have data, or have an
    /// in-flight `MemReq` whose response will carry pre-store memory bytes).
    /// Returns the oldest violating load's `rob_tag`, if any.
    ///
    /// Translated loads must be flagged too, not just Executed ones: with a
    /// Bypass prediction, a load can issue its `MemReq` before an older store
    /// to the same address resolves. If we wait until the load's response
    /// arrives (Executed) to detect the conflict, the store's check has
    /// already run and missed it, and the load commits with stale data.
    pub fn check_ordering_violation(
        &self,
        store_paddr: PhysAddr,
        store_width: MemWidth,
        store_rob_tag: RobTag,
    ) -> Option<RobTag> {
        let store_bytes = width_to_bytes(store_width) as u64;
        self.check_ordering_violation_over(store_paddr, store_bytes, store_rob_tag)
    }

    /// [`Self::check_ordering_violation`] for a store of `store_bytes`
    /// bytes, such as a cache-block operation on a whole block.
    pub fn check_ordering_violation_over(
        &self,
        store_paddr: PhysAddr,
        store_bytes: u64,
        store_rob_tag: RobTag,
    ) -> Option<RobTag> {
        let store_start = store_paddr.val();
        let store_end = store_start + store_bytes;

        let mut oldest_violator: Option<RobTag> = None;
        for entry in &self.entries {
            if !entry.valid
                || !entry.rob_tag.is_newer_than(store_rob_tag)
                || matches!(entry.state, LoadState::Pending)
            {
                continue;
            }
            let Some(load_paddr) = entry.paddr else { continue };
            let load_size = entry.bytes as u64;
            let load_start = load_paddr.val();
            let load_end = load_start + load_size;
            if load_start < store_end && load_end > store_start {
                match oldest_violator {
                    None => oldest_violator = Some(entry.rob_tag),
                    Some(prev) if entry.rob_tag.is_older_than(prev) => {
                        oldest_violator = Some(entry.rob_tag);
                    }
                    _ => {}
                }
            }
        }
        oldest_violator
    }

    /// Coherence check when the load `older_tag` receives its value for
    /// `paddr`: a younger load to the same line that already executed and
    /// whose value another hart has since overwritten read the line's
    /// earlier value, which the older load can no longer observe. Returns
    /// the oldest such load so the caller can squash from it. This is the
    /// rule gem5's LSQ applies on an external snoop.
    pub fn check_coherence_violation(
        &self,
        older_tag: RobTag,
        paddr: PhysAddr,
        log: &WriteLog,
        reader: HartId,
    ) -> Option<RobTag> {
        let line_bytes = log.line_bytes();
        let line = paddr.val() / line_bytes;
        let mut oldest: Option<RobTag> = None;
        for entry in &self.entries {
            if !entry.valid
                || !entry.rob_tag.is_newer_than(older_tag)
                || entry.state != LoadState::Executed
            {
                continue;
            }
            let (Some(entry_paddr), Some(observed)) = (entry.paddr, entry.observed) else {
                continue;
            };
            if entry_paddr.val() / line_bytes != line
                || !log.written_by_other_since(entry_paddr, reader, observed)
            {
                continue;
            }
            match oldest {
                Some(prev) if !entry.rob_tag.is_older_than(prev) => {}
                _ => oldest = Some(entry.rob_tag),
            }
        }
        oldest
    }

    /// Deallocates all load queue entries with the given ROB tag.
    pub fn deallocate(&mut self, rob_tag: RobTag) {
        for entry in &mut self.entries {
            if entry.valid && entry.rob_tag == rob_tag {
                entry.valid = false;
                self.valid_count -= 1;
            }
        }
    }

    /// Deallocates the entry of one vector load micro-op, which is released
    /// at its writeback (wave-based reclaim).
    pub fn deallocate_micro_op(&mut self, rob_tag: RobTag, micro_op: MicroOpIdx) {
        for entry in &mut self.entries {
            if entry.valid && entry.rob_tag == rob_tag && entry.micro_op == Some(micro_op) {
                entry.valid = false;
                self.valid_count -= 1;
                return;
            }
        }
    }

    /// Flushes all entries (trap / full pipeline flush).
    pub fn flush(&mut self) {
        for entry in &mut self.entries {
            entry.valid = false;
        }
        self.valid_count = 0;
    }

    /// Flushes entries newer than `keep_tag` (misprediction recovery).
    pub fn flush_after(&mut self, keep_tag: RobTag) {
        for entry in &mut self.entries {
            if entry.valid && entry.rob_tag.is_newer_than(keep_tag) {
                entry.valid = false;
                self.valid_count -= 1;
            }
        }
    }

    fn find_mut(
        &mut self,
        rob_tag: RobTag,
        micro_op: Option<MicroOpIdx>,
    ) -> Option<&mut LoadQueueEntry> {
        self.entries.iter_mut().find(|e| e.valid && e.rob_tag == rob_tag && e.micro_op == micro_op)
    }
}

#[cfg(test)]
mod coherence_tests {
    use super::*;
    use crate::sim::memory::write_log::Writer;

    const H0: HartId = HartId::new(0);
    const H1: HartId = HartId::new(1);

    fn executed_load(lq: &mut LoadQueue, tag: RobTag, paddr: u64, observed: WriteSeq) {
        assert!(lq.allocate(tag, 8, None));
        lq.fill_address(tag, None, VirtAddr::new(paddr), PhysAddr::new(paddr));
        lq.fill_data(tag, None, 0, Some(observed));
    }

    #[test]
    fn a_younger_load_that_read_before_a_remote_write_is_squashed() {
        let mut lq = LoadQueue::new(4);
        let mut log = WriteLog::new(0x8000_0000, 0x1000, 64, 2);
        executed_load(&mut lq, RobTag(2), 0x8000_0100, log.now());
        log.record(PhysAddr::new(0x8000_0108), Writer::Hart(H1));

        let violator =
            lq.check_coherence_violation(RobTag(1), PhysAddr::new(0x8000_0120), &log, H0);

        assert_eq!(violator, Some(RobTag(2)));
    }

    #[test]
    fn the_oldest_violating_load_is_reported() {
        let mut lq = LoadQueue::new(4);
        let mut log = WriteLog::new(0x8000_0000, 0x1000, 64, 2);
        executed_load(&mut lq, RobTag(3), 0x8000_0100, log.now());
        executed_load(&mut lq, RobTag(2), 0x8000_0110, log.now());
        log.record(PhysAddr::new(0x8000_0100), Writer::Hart(H1));

        assert_eq!(
            lq.check_coherence_violation(RobTag(1), PhysAddr::new(0x8000_0100), &log, H0),
            Some(RobTag(2))
        );
    }

    #[test]
    fn loads_to_other_lines_or_older_than_the_reader_are_ignored() {
        let mut lq = LoadQueue::new(4);
        let mut log = WriteLog::new(0x8000_0000, 0x1000, 64, 2);
        executed_load(&mut lq, RobTag(2), 0x8000_0100, log.now());
        executed_load(&mut lq, RobTag(0), 0x8000_0140, log.now());
        log.record(PhysAddr::new(0x8000_0100), Writer::Hart(H1));
        log.record(PhysAddr::new(0x8000_0140), Writer::Hart(H1));

        assert_eq!(
            lq.check_coherence_violation(RobTag(1), PhysAddr::new(0x8000_0200), &log, H0),
            None,
            "different line"
        );
        assert_eq!(
            lq.check_coherence_violation(RobTag(1), PhysAddr::new(0x8000_0140), &log, H0),
            None,
            "tag 0 is older than the reader"
        );
    }

    #[test]
    fn a_younger_load_stamped_after_the_write_is_consistent() {
        let mut lq = LoadQueue::new(4);
        let mut log = WriteLog::new(0x8000_0000, 0x1000, 64, 2);
        log.record(PhysAddr::new(0x8000_0100), Writer::Hart(H1));
        executed_load(&mut lq, RobTag(2), 0x8000_0100, log.now());

        assert_eq!(
            lq.check_coherence_violation(RobTag(1), PhysAddr::new(0x8000_0100), &log, H0),
            None
        );
    }

    #[test]
    fn the_readers_own_writes_never_squash() {
        let mut lq = LoadQueue::new(4);
        let mut log = WriteLog::new(0x8000_0000, 0x1000, 64, 2);
        executed_load(&mut lq, RobTag(2), 0x8000_0100, log.now());
        log.record(PhysAddr::new(0x8000_0100), Writer::Hart(H0));

        assert_eq!(
            lq.check_coherence_violation(RobTag(1), PhysAddr::new(0x8000_0100), &log, H0),
            None
        );
    }
}

/// Converts a `MemWidth` to byte count.
const fn width_to_bytes(w: MemWidth) -> usize {
    match w {
        MemWidth::Byte => 1,
        MemWidth::Half => 2,
        MemWidth::Word => 4,
        MemWidth::Double => 8,
        MemWidth::Nop => 0,
    }
}

#[cfg(test)]
#[allow(unused_results)]
mod tests {
    use super::*;

    #[test]
    fn allocate_and_deallocate() {
        let mut lq = LoadQueue::new(4);
        assert!(lq.is_empty());

        let tag = RobTag(1);
        assert!(lq.allocate(tag, 4, None));
        assert_eq!(lq.len(), 1);

        lq.fill_address(tag, None, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000));
        lq.fill_data(tag, None, 0xDEADBEEF, None);

        lq.deallocate(tag);
        assert!(lq.is_empty());
    }

    #[test]
    fn full_queue() {
        let mut lq = LoadQueue::new(2);
        assert!(lq.allocate(RobTag(1), 4, None));
        assert!(lq.allocate(RobTag(2), 4, None));
        assert!(lq.is_full());
        assert!(!lq.allocate(RobTag(3), 4, None));
    }

    #[test]
    fn a_store_over_one_micro_op_of_a_load_is_a_violation() {
        let mut lq = LoadQueue::new(4);
        let load = RobTag(2);
        let (first, second) = (MicroOpIdx::new(0), MicroOpIdx::new(1));
        lq.allocate(load, 4, Some(first));
        lq.allocate(load, 4, Some(second));
        lq.fill_address(load, Some(first), VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000));
        lq.fill_address(load, Some(second), VirtAddr::new(0x1004), PhysAddr::new(0x8000_0004));

        let violation =
            lq.check_ordering_violation(PhysAddr::new(0x8000_0004), MemWidth::Word, RobTag(1));

        assert_eq!(violation, Some(load));
    }

    #[test]
    fn a_load_is_checked_over_all_its_bytes() {
        let mut lq = LoadQueue::new(4);
        let load = RobTag(2);
        lq.allocate(load, 32, Some(MicroOpIdx::new(0)));
        lq.fill_address(
            load,
            Some(MicroOpIdx::new(0)),
            VirtAddr::new(0x1000),
            PhysAddr::new(0x8000_0000),
        );

        let violation =
            lq.check_ordering_violation(PhysAddr::new(0x8000_001C), MemWidth::Word, RobTag(1));

        assert_eq!(violation, Some(load));
    }

    #[test]
    fn deallocate_micro_op_reuses_slot_after_middle_invalidation() {
        // Out-of-order completion: invalidate a middle entry, then allocate.
        // The freed slot must be reusable. (Regression: the previous circular
        // FIFO leaked middle slots and deadlocked vec-segment loads.)
        let mut lq = LoadQueue::new(3);
        lq.allocate(RobTag(1), 4, Some(MicroOpIdx::new(0)));
        lq.allocate(RobTag(1), 4, Some(MicroOpIdx::new(1)));
        lq.allocate(RobTag(1), 4, Some(MicroOpIdx::new(2)));
        assert!(lq.is_full());

        // Free the middle entry, not the head.
        lq.deallocate_micro_op(RobTag(1), MicroOpIdx::new(1));
        assert!(!lq.is_full());
        assert_eq!(lq.free_slots(), 1);

        // The freed slot must be reusable.
        assert!(lq.allocate(RobTag(2), 4, Some(MicroOpIdx::new(0))));
    }

    #[test]
    fn ordering_violation() {
        let mut lq = LoadQueue::new(4);

        // Younger load (tag=3) executes before older store (tag=2) resolves
        let load_tag = RobTag(3);
        lq.allocate(load_tag, 4, None);
        lq.fill_address(load_tag, None, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000));
        lq.fill_data(load_tag, None, 0x12345678, None);

        // Store (tag=2) resolves to same address — violation!
        let result =
            lq.check_ordering_violation(PhysAddr::new(0x8000_0000), MemWidth::Word, RobTag(2));
        assert_eq!(result, Some(RobTag(3)));
    }

    #[test]
    fn no_violation_different_address() {
        let mut lq = LoadQueue::new(4);

        let load_tag = RobTag(3);
        lq.allocate(load_tag, 4, None);
        lq.fill_address(load_tag, None, VirtAddr::new(0x2000), PhysAddr::new(0x8000_0004));
        lq.fill_data(load_tag, None, 0x12345678, None);

        let result =
            lq.check_ordering_violation(PhysAddr::new(0x8000_0000), MemWidth::Word, RobTag(2));
        assert_eq!(result, None);
    }

    #[test]
    fn no_violation_older_load() {
        let mut lq = LoadQueue::new(4);

        let load_tag = RobTag(1);
        lq.allocate(load_tag, 4, None);
        lq.fill_address(load_tag, None, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000));
        lq.fill_data(load_tag, None, 0x12345678, None);

        let result =
            lq.check_ordering_violation(PhysAddr::new(0x8000_0000), MemWidth::Word, RobTag(2));
        assert_eq!(result, None);
    }

    #[test]
    fn flush_after_keeps_older() {
        let mut lq = LoadQueue::new(4);
        lq.allocate(RobTag(1), 4, None);
        lq.allocate(RobTag(2), 4, None);
        lq.allocate(RobTag(3), 4, None);

        lq.flush_after(RobTag(1));
        assert_eq!(lq.len(), 1);
    }

    #[test]
    fn flush_clears_all() {
        let mut lq = LoadQueue::new(4);
        lq.allocate(RobTag(1), 4, None);
        lq.allocate(RobTag(2), 4, None);

        lq.flush();
        assert!(lq.is_empty());
    }

    #[test]
    fn capacity_two_repeatedly_reused() {
        let mut lq = LoadQueue::new(2);
        for i in 1..=10 {
            let tag = RobTag(i);
            assert!(lq.allocate(tag, 4, None));
            lq.fill_address(tag, None, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000));
            lq.fill_data(tag, None, i as u64, None);
            lq.deallocate(tag);
        }
    }
}
