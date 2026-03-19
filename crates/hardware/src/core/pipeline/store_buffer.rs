//! Store Buffer for deferred memory writes.
//!
//! Stores are not written to memory until they commit from the ROB. The store
//! buffer holds pending stores and provides:
//! 1. **Allocation:** Reserve a slot when a store enters the backend.
//! 2. **Resolution:** Fill in the physical address and data after Memory1/Memory2.
//! 3. **Forwarding:** Provide store-to-load forwarding for loads that hit a pending store.
//! 4. **Commit:** Mark entries as committed when the ROB retires the store.
//! 5. **Drain:** Write committed stores to memory one per cycle.

use crate::common::{PhysAddr, VirtAddr};
use crate::core::pipeline::rob::RobTag;
use crate::core::pipeline::signals::MemWidth;

/// Result of store-to-load forwarding check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardResult {
    /// Store fully covers the load — use the forwarded data.
    Hit(u64),
    /// No overlap with any pending store — safe to read from memory.
    Miss,
    /// Partial overlap — must stall until the store drains to memory.
    Stall,
}

/// Lifecycle state of a store buffer entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StoreState {
    /// Allocated but address/data not yet resolved.
    #[default]
    Pending,
    /// Address and data resolved, waiting for ROB commit.
    Ready,
    /// ROB has committed this store; it can be drained to memory.
    Committed,
}

/// A single entry in the store buffer.
#[derive(Clone, Debug, Default)]
pub struct StoreBufferEntry {
    /// ROB tag of the store instruction.
    pub rob_tag: RobTag,
    /// Virtual address of the store.
    pub vaddr: VirtAddr,
    /// Physical address (filled after translation).
    pub paddr: Option<PhysAddr>,
    /// Data to store.
    pub data: u64,
    /// Width of the store operation.
    pub width: MemWidth,
    /// Current lifecycle state.
    pub state: StoreState,
    /// Whether this slot is occupied.
    pub valid: bool,
}

/// Store buffer — FIFO queue of pending stores.
#[derive(Debug)]
pub struct StoreBuffer {
    entries: Vec<StoreBufferEntry>,
    /// Index of the oldest entry.
    head: usize,
    /// Index where the next entry will be allocated.
    tail: usize,
    /// Number of valid entries.
    count: usize,
}

impl StoreBuffer {
    /// Creates a new store buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let mut entries = Vec::with_capacity(capacity);
        entries.resize_with(capacity, StoreBufferEntry::default);
        Self { entries, head: 0, tail: 0, count: 0 }
    }

    /// Returns the capacity.
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Returns the number of occupied entries.
    #[inline]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Returns true if the store buffer is empty.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns true if any committed stores are waiting to drain to RAM.
    ///
    /// Unlike `is_empty()`, this ignores speculative (Pending/Ready) entries.
    /// Used by the SFENCE.VMA stall: the fence only needs to wait for
    /// committed stores to reach RAM — younger speculative entries will be
    /// squashed by the full pipeline flush after the fence commits.
    pub fn has_committed_stores(&self) -> bool {
        if self.count == 0 {
            return false;
        }
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.state == StoreState::Committed {
                return true;
            }
            idx = (idx + 1) % cap;
        }
        false
    }

    /// Returns true if the store buffer is full.
    #[inline]
    pub const fn is_full(&self) -> bool {
        self.count == self.entries.len()
    }

    /// Returns the number of free slots.
    #[inline]
    pub const fn free_slots(&self) -> usize {
        self.entries.len() - self.count
    }

    /// Allocates a slot for a new store. Returns false if the buffer is full.
    pub fn allocate(&mut self, rob_tag: RobTag, width: MemWidth) -> bool {
        if self.is_full() {
            return false;
        }

        self.entries[self.tail] = StoreBufferEntry {
            rob_tag,
            vaddr: VirtAddr::new(0),
            paddr: None,
            data: 0,
            width,
            state: StoreState::Pending,
            valid: true,
        };

        self.tail = (self.tail + 1) % self.entries.len();
        self.count += 1;
        true
    }

    /// Resolves a store's address and data after memory translation.
    pub fn resolve(&mut self, rob_tag: RobTag, vaddr: VirtAddr, paddr: PhysAddr, data: u64) {
        if let Some(entry) = self.find_by_tag_mut(rob_tag) {
            entry.vaddr = vaddr;
            entry.paddr = Some(paddr);
            entry.data = data;
            entry.state = StoreState::Ready;
        }
    }

    /// Marks a store as committed (the ROB has retired the instruction).
    pub fn mark_committed(&mut self, rob_tag: RobTag) {
        if let Some(entry) = self.find_by_tag_mut(rob_tag)
            && entry.state == StoreState::Ready
        {
            entry.state = StoreState::Committed;
        }
    }

    /// Attempts store-to-load forwarding.
    ///
    /// Returns `Hit(data)` if a pending store fully covers the load,
    /// `Stall` if a store partially overlaps (must wait for drain),
    /// or `Miss` if no overlap exists.
    ///
    /// `load_rob_tag` is the ROB tag of the load instruction. Only stores
    /// older than the load (lower tag) are considered for forwarding. Stores
    /// newer than the load in program order are skipped.
    pub fn forward_load(
        &self,
        paddr: PhysAddr,
        width: MemWidth,
        load_rob_tag: RobTag,
    ) -> ForwardResult {
        let load_size = width_to_bytes(width);
        let load_start = paddr.val();
        let load_end = load_start + load_size as u64;

        // Search from newest to oldest for the most recent matching store
        let mut idx = if self.tail == 0 { self.entries.len() - 1 } else { self.tail - 1 };

        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid {
                // Skip stores that are newer than or same age as the load —
                // they are after the load in program order and must not forward.
                if !entry.rob_tag.is_older_than(load_rob_tag) {
                    if idx == 0 {
                        idx = self.entries.len() - 1;
                    } else {
                        idx -= 1;
                    }
                    continue;
                }

                // Older store with resolved address — check for overlap.
                if let Some(store_paddr) = entry.paddr {
                    let store_size = width_to_bytes(entry.width);
                    let store_start = store_paddr.val();
                    let store_end = store_start + store_size as u64;

                    // Check for any overlap
                    if load_start < store_end && load_end > store_start {
                        // Full overlap: store completely covers the load
                        if store_start <= load_start && store_end >= load_end {
                            let offset = (load_start - store_start) as u32;
                            let shifted = entry.data >> (offset * 8);
                            let mask = if load_size >= 8 {
                                u64::MAX
                            } else {
                                (1u64 << (load_size * 8)) - 1
                            };
                            return ForwardResult::Hit(shifted & mask);
                        }
                        // Partial overlap: must stall
                        return ForwardResult::Stall;
                    }
                }
                // Older store with paddr=None: skip (handled by issue-time
                // ordering — loads are not issued until all older stores have
                // their addresses resolved).
            }
            if idx == 0 {
                idx = self.entries.len() - 1;
            } else {
                idx -= 1;
            }
        }

        ForwardResult::Miss
    }

    /// Checks whether any store buffer entry older than `rob_tag` has an
    /// unresolved address (paddr=None). Used by the issue queue to prevent
    /// loads from issuing before older stores have their addresses resolved.
    pub fn has_unresolved_store_before(&self, rob_tag: RobTag) -> bool {
        if self.count == 0 {
            return false;
        }
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            // An entry is truly unresolved only if it's Pending (address
            // computation hasn't happened yet). Cancelled SC entries have
            // paddr=None but state=Committed — they are harmless no-ops
            // waiting to drain and must not block younger loads.
            if entry.valid
                && entry.rob_tag.is_older_than(rob_tag)
                && entry.paddr.is_none()
                && entry.state == StoreState::Pending
            {
                return true;
            }
            idx = (idx + 1) % cap;
        }
        false
    }

    /// Checks whether any store buffer entry older than `rob_tag` overlaps
    /// the given physical address range. Used by LR/AMO to stall until older
    /// stores to the same address have drained, preserving atomicity.
    pub fn has_older_store_to(&self, paddr: PhysAddr, width: MemWidth, rob_tag: RobTag) -> bool {
        if self.count == 0 {
            return false;
        }
        let load_size = width_to_bytes(width) as u64;
        let load_start = paddr.val();
        let load_end = load_start + load_size;
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.rob_tag.is_older_than(rob_tag) {
                // Unresolved store to unknown address — must assume overlap
                if entry.paddr.is_none() && entry.state == StoreState::Pending {
                    return true;
                }
                // Resolved store — check for address overlap
                if let Some(store_paddr) = entry.paddr {
                    let store_size = width_to_bytes(entry.width) as u64;
                    let store_start = store_paddr.val();
                    let store_end = store_start + store_size;
                    if load_start < store_end && load_end > store_start {
                        return true;
                    }
                }
            }
            idx = (idx + 1) % cap;
        }
        false
    }

    /// Drains (removes) the oldest committed store. Returns it so the caller
    /// can write it to memory. Returns `None` if no committed store is available.
    pub fn drain_one(&mut self) -> Option<StoreBufferEntry> {
        if self.count == 0 {
            return None;
        }

        let entry = &self.entries[self.head];
        if !entry.valid || entry.state != StoreState::Committed {
            return None;
        }

        let drained = self.entries[self.head].clone();
        self.entries[self.head].valid = false;
        self.head = (self.head + 1) % self.entries.len();
        self.count -= 1;
        Some(drained)
    }

    /// Flushes speculative (non-committed) entries. Committed entries remain.
    pub fn flush_speculative(&mut self) {
        if self.count == 0 {
            return;
        }

        // Walk from head to tail, keep only Committed entries at the front
        let cap = self.entries.len();
        let mut new_tail = self.head;
        let mut new_count = 0;
        let mut idx = self.head;

        for _ in 0..self.count {
            if self.entries[idx].valid && self.entries[idx].state == StoreState::Committed {
                if idx != new_tail {
                    self.entries[new_tail] = self.entries[idx].clone();
                    self.entries[idx].valid = false;
                }
                new_tail = (new_tail + 1) % cap;
                new_count += 1;
            } else {
                self.entries[idx].valid = false;
            }
            idx = (idx + 1) % cap;
        }

        self.tail = new_tail;
        self.count = new_count;
    }

    /// Flushes store buffer entries allocated *after* the given ROB tag.
    ///
    /// Entries with tags up to and including `keep_tag` are retained (whether
    /// Pending, Ready, or Committed). Only entries whose ROB tag is strictly
    /// newer than `keep_tag` are removed.
    ///
    /// This is used on branch mispredictions where pre-branch stores that are
    /// still in-flight (Ready but not yet Committed) must be kept.
    pub fn flush_after(&mut self, keep_tag: RobTag) {
        if self.count == 0 {
            return;
        }

        let cap = self.entries.len();
        let mut new_tail = self.head;
        let mut new_count = 0;
        let mut idx = self.head;

        for _ in 0..self.count {
            let entry = &self.entries[idx];
            // Keep entries that are at or before the keep_tag in program order.
            // ROB tags are assigned sequentially, so tag <= keep_tag means
            // the instruction was issued at or before the branch.
            if entry.valid && entry.rob_tag.is_older_or_eq(keep_tag) {
                if idx != new_tail {
                    self.entries[new_tail] = self.entries[idx].clone();
                    self.entries[idx].valid = false;
                }
                new_tail = (new_tail + 1) % cap;
                new_count += 1;
            } else {
                self.entries[idx].valid = false;
            }
            idx = (idx + 1) % cap;
        }

        self.tail = new_tail;
        self.count = new_count;
    }

    /// Flushes all entries (including committed ones).
    pub fn flush_all(&mut self) {
        for entry in &mut self.entries {
            entry.valid = false;
        }
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }

    /// Cancels (removes) a store buffer entry that will not be written.
    /// Used for failed SC (store-conditional) instructions.
    pub fn cancel(&mut self, rob_tag: RobTag) {
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            if self.entries[idx].valid && self.entries[idx].rob_tag == rob_tag {
                // If this is the tail entry, we can simply retract it
                let prev_tail = if self.tail == 0 { cap - 1 } else { self.tail - 1 };
                if idx == prev_tail {
                    self.entries[idx].valid = false;
                    self.tail = prev_tail;
                    self.count -= 1;
                } else {
                    // Not at tail — resolve as a committed no-op that drain_one will skip.
                    // Mark it Committed so it can drain, and clear paddr so
                    // drain_one's `let Some(paddr) = store.paddr` guard skips the write.
                    self.entries[idx].state = StoreState::Committed;
                    self.entries[idx].paddr = None;
                }
                return;
            }
            idx = (idx + 1) % cap;
        }
    }

    /// Returns the resolved physical address for the entry with the given ROB tag,
    /// or `None` if the entry is not found or has no address yet.
    pub fn find_paddr(&self, rob_tag: RobTag) -> Option<PhysAddr> {
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            if self.entries[idx].valid && self.entries[idx].rob_tag == rob_tag {
                return self.entries[idx].paddr;
            }
            idx = (idx + 1) % cap;
        }
        None
    }

    /// Finds the entry with the given ROB tag.
    fn find_by_tag_mut(&mut self, rob_tag: RobTag) -> Option<&mut StoreBufferEntry> {
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            if self.entries[idx].valid && self.entries[idx].rob_tag == rob_tag {
                return Some(&mut self.entries[idx]);
            }
            idx = (idx + 1) % cap;
        }
        None
    }
}

/// Converts a `MemWidth` to byte count.
pub(crate) const fn width_to_bytes(w: MemWidth) -> usize {
    match w {
        MemWidth::Byte => 1,
        MemWidth::Half => 2,
        MemWidth::Word => 4,
        MemWidth::Double => 8,
        MemWidth::Nop => 0,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_and_drain() {
        let mut sb = StoreBuffer::new(4);
        assert!(sb.is_empty());

        let tag = RobTag(1);
        assert!(sb.allocate(tag, MemWidth::Word));
        assert_eq!(sb.len(), 1);

        // Can't drain yet (still Pending)
        assert!(sb.drain_one().is_none());

        sb.resolve(tag, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000), 0xDEADBEEF);
        // Can't drain yet (Ready but not Committed)
        assert!(sb.drain_one().is_none());

        sb.mark_committed(tag);
        let entry = sb.drain_one().unwrap();
        assert_eq!(entry.paddr, Some(PhysAddr::new(0x8000_0000)));
        assert_eq!(entry.data, 0xDEADBEEF);
        assert!(sb.is_empty());
    }

    #[test]
    fn test_full_buffer() {
        let mut sb = StoreBuffer::new(2);
        assert!(sb.allocate(RobTag(1), MemWidth::Word));
        assert!(sb.allocate(RobTag(2), MemWidth::Word));
        assert!(sb.is_full());
        assert!(!sb.allocate(RobTag(3), MemWidth::Word));
    }

    #[test]
    fn test_forward_load() {
        let mut sb = StoreBuffer::new(4);
        let tag = RobTag(1);
        sb.allocate(tag, MemWidth::Word);
        sb.resolve(tag, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000), 0x12345678);

        // Forward should find the store (load is younger: tag 2 > store tag 1)
        let result = sb.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Word, RobTag(2));
        assert_eq!(result, ForwardResult::Hit(0x12345678));

        // Different address should miss
        let result = sb.forward_load(PhysAddr::new(0x8000_0004), MemWidth::Word, RobTag(2));
        assert_eq!(result, ForwardResult::Miss);
    }

    #[test]
    fn test_forward_load_byte() {
        let mut sb = StoreBuffer::new(4);
        let tag = RobTag(1);
        sb.allocate(tag, MemWidth::Word);
        sb.resolve(tag, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000), 0x12345678);

        // Forward a byte from the same address
        let result = sb.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag(2));
        assert_eq!(result, ForwardResult::Hit(0x78));
    }

    #[test]
    fn test_flush_speculative() {
        let mut sb = StoreBuffer::new(4);
        let t1 = RobTag(1);
        let t2 = RobTag(2);
        let t3 = RobTag(3);

        sb.allocate(t1, MemWidth::Word);
        sb.allocate(t2, MemWidth::Word);
        sb.allocate(t3, MemWidth::Word);

        sb.resolve(t1, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000), 10);
        sb.mark_committed(t1);

        sb.resolve(t2, VirtAddr::new(0x1004), PhysAddr::new(0x8000_0004), 20);
        // t2 is Ready but not committed
        // t3 is still Pending

        sb.flush_speculative();
        assert_eq!(sb.len(), 1); // only t1 remains

        let entry = sb.drain_one().unwrap();
        assert_eq!(entry.data, 10);
    }

    #[test]
    fn test_flush_all() {
        let mut sb = StoreBuffer::new(4);
        sb.allocate(RobTag(1), MemWidth::Word);
        sb.allocate(RobTag(2), MemWidth::Word);

        sb.flush_all();
        assert!(sb.is_empty());
    }

    #[test]
    fn test_circular_wraparound() {
        let mut sb = StoreBuffer::new(2);
        for i in 1..=10 {
            let tag = RobTag(i);
            sb.allocate(tag, MemWidth::Word);
            sb.resolve(tag, VirtAddr::new(0), PhysAddr::new(0x8000_0000), i as u64);
            sb.mark_committed(tag);
            let entry = sb.drain_one().unwrap();
            assert_eq!(entry.data, i as u64);
        }
    }
}
