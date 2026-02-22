//! Load Queue for in-flight load tracking and memory ordering violation detection.
//!
//! Tracks pending loads and detects memory ordering violations when a store
//! resolves its address and overlaps with a younger load that has already
//! executed with potentially stale data. Same circular FIFO design as StoreBuffer.

use crate::core::pipeline::rob::RobTag;
use crate::core::pipeline::signals::MemWidth;

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
    pub vaddr: u64,
    /// Physical address (filled after translation).
    pub paddr: Option<u64>,
    /// Data read from memory.
    pub data: u64,
    /// Width of the load operation.
    pub width: MemWidth,
    /// Current lifecycle state.
    pub state: LoadState,
    /// Whether this slot is occupied.
    pub valid: bool,
}

/// Load queue — FIFO queue of pending loads.
pub struct LoadQueue {
    entries: Vec<LoadQueueEntry>,
    /// Index of the oldest entry.
    head: usize,
    /// Index where the next entry will be allocated.
    tail: usize,
    /// Number of valid entries.
    count: usize,
}

impl LoadQueue {
    /// Creates a new load queue with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let mut entries = Vec::with_capacity(capacity);
        entries.resize_with(capacity, LoadQueueEntry::default);
        Self {
            entries,
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    /// Returns the capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Returns the number of occupied entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns true if the load queue is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns true if the load queue is full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.count == self.entries.len()
    }

    /// Returns the number of free slots.
    #[inline]
    pub fn free_slots(&self) -> usize {
        self.entries.len() - self.count
    }

    /// Allocates a slot for a new load. Returns false if the buffer is full.
    pub fn allocate(&mut self, rob_tag: RobTag, width: MemWidth) -> bool {
        if self.is_full() {
            return false;
        }

        self.entries[self.tail] = LoadQueueEntry {
            rob_tag,
            vaddr: 0,
            paddr: None,
            data: 0,
            width,
            state: LoadState::Pending,
            valid: true,
        };

        self.tail = (self.tail + 1) % self.entries.len();
        self.count += 1;
        true
    }

    /// Fills the translated address for a load after Memory1.
    pub fn fill_address(&mut self, rob_tag: RobTag, vaddr: u64, paddr: u64) {
        if let Some(entry) = self.find_by_tag_mut(rob_tag) {
            entry.vaddr = vaddr;
            entry.paddr = Some(paddr);
            entry.state = LoadState::Translated;
        }
    }

    /// Fills the loaded data for a load after Memory2.
    pub fn fill_data(&mut self, rob_tag: RobTag, data: u64) {
        if let Some(entry) = self.find_by_tag_mut(rob_tag) {
            entry.data = data;
            entry.state = LoadState::Executed;
        }
    }

    /// Checks for memory ordering violations when a store resolves its address.
    ///
    /// Scans for younger loads (rob_tag > store_rob_tag) that have already
    /// executed and overlap with the store's address range. Returns the oldest
    /// violating load's rob_tag, if any.
    pub fn check_ordering_violation(
        &self,
        store_paddr: u64,
        store_width: MemWidth,
        store_rob_tag: RobTag,
    ) -> Option<RobTag> {
        if self.count == 0 {
            return None;
        }

        let store_size = width_to_bytes(store_width) as u64;
        let store_start = store_paddr;
        let store_end = store_paddr + store_size;

        let cap = self.entries.len();
        let mut idx = self.head;
        let mut oldest_violator: Option<RobTag> = None;

        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid
                && entry.rob_tag.0 > store_rob_tag.0
                && entry.state == LoadState::Executed
            {
                if let Some(load_paddr) = entry.paddr {
                    let load_size = width_to_bytes(entry.width) as u64;
                    let load_start = load_paddr;
                    let load_end = load_paddr + load_size;

                    // Check for any overlap
                    if load_start < store_end && load_end > store_start {
                        match oldest_violator {
                            None => oldest_violator = Some(entry.rob_tag),
                            Some(prev) if entry.rob_tag.0 < prev.0 => {
                                oldest_violator = Some(entry.rob_tag);
                            }
                            _ => {}
                        }
                    }
                }
            }
            idx = (idx + 1) % cap;
        }

        oldest_violator
    }

    /// Deallocates the oldest load (at head). Called when a load commits.
    pub fn deallocate(&mut self, rob_tag: RobTag) {
        if self.count == 0 {
            return;
        }

        // The committed load should be at the head (in-order commit).
        if self.entries[self.head].valid && self.entries[self.head].rob_tag == rob_tag {
            self.entries[self.head].valid = false;
            self.head = (self.head + 1) % self.entries.len();
            self.count -= 1;
            return;
        }

        // Fallback: search for it (shouldn't normally happen with in-order commit).
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            if self.entries[idx].valid && self.entries[idx].rob_tag == rob_tag {
                self.entries[idx].valid = false;
                // Don't adjust head/tail — just mark invalid.
                // Advance head past any invalid entries at the front.
                while self.count > 0 && !self.entries[self.head].valid {
                    self.head = (self.head + 1) % cap;
                    self.count -= 1;
                }
                return;
            }
            idx = (idx + 1) % cap;
        }
    }

    /// Flushes all entries (trap / full pipeline flush).
    pub fn flush(&mut self) {
        for entry in &mut self.entries {
            entry.valid = false;
        }
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }

    /// Flushes entries newer than `keep_tag` (misprediction recovery).
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
            if entry.valid && entry.rob_tag.0 <= keep_tag.0 {
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

    /// Finds the entry with the given ROB tag.
    fn find_by_tag_mut(&mut self, rob_tag: RobTag) -> Option<&mut LoadQueueEntry> {
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

/// Converts a MemWidth to byte count.
fn width_to_bytes(w: MemWidth) -> usize {
    match w {
        MemWidth::Byte => 1,
        MemWidth::Half => 2,
        MemWidth::Word => 4,
        MemWidth::Double => 8,
        MemWidth::Nop => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_and_deallocate() {
        let mut lq = LoadQueue::new(4);
        assert!(lq.is_empty());

        let tag = RobTag(1);
        assert!(lq.allocate(tag, MemWidth::Word));
        assert_eq!(lq.len(), 1);

        lq.fill_address(tag, 0x1000, 0x8000_0000);
        lq.fill_data(tag, 0xDEADBEEF);

        lq.deallocate(tag);
        assert!(lq.is_empty());
    }

    #[test]
    fn test_full_queue() {
        let mut lq = LoadQueue::new(2);
        assert!(lq.allocate(RobTag(1), MemWidth::Word));
        assert!(lq.allocate(RobTag(2), MemWidth::Word));
        assert!(lq.is_full());
        assert!(!lq.allocate(RobTag(3), MemWidth::Word));
    }

    #[test]
    fn test_ordering_violation() {
        let mut lq = LoadQueue::new(4);

        // Younger load (tag=3) executes before older store (tag=2) resolves
        let load_tag = RobTag(3);
        lq.allocate(load_tag, MemWidth::Word);
        lq.fill_address(load_tag, 0x1000, 0x8000_0000);
        lq.fill_data(load_tag, 0x12345678);

        // Store (tag=2) resolves to same address — violation!
        let result = lq.check_ordering_violation(0x8000_0000, MemWidth::Word, RobTag(2));
        assert_eq!(result, Some(RobTag(3)));
    }

    #[test]
    fn test_no_violation_different_address() {
        let mut lq = LoadQueue::new(4);

        let load_tag = RobTag(3);
        lq.allocate(load_tag, MemWidth::Word);
        lq.fill_address(load_tag, 0x2000, 0x8000_0004);
        lq.fill_data(load_tag, 0x12345678);

        // Store to different address — no violation
        let result = lq.check_ordering_violation(0x8000_0000, MemWidth::Word, RobTag(2));
        assert_eq!(result, None);
    }

    #[test]
    fn test_no_violation_older_load() {
        let mut lq = LoadQueue::new(4);

        // Load is older than store — no violation (correct ordering)
        let load_tag = RobTag(1);
        lq.allocate(load_tag, MemWidth::Word);
        lq.fill_address(load_tag, 0x1000, 0x8000_0000);
        lq.fill_data(load_tag, 0x12345678);

        let result = lq.check_ordering_violation(0x8000_0000, MemWidth::Word, RobTag(2));
        assert_eq!(result, None);
    }

    #[test]
    fn test_flush_after() {
        let mut lq = LoadQueue::new(4);
        lq.allocate(RobTag(1), MemWidth::Word);
        lq.allocate(RobTag(2), MemWidth::Word);
        lq.allocate(RobTag(3), MemWidth::Word);

        lq.flush_after(RobTag(1));
        assert_eq!(lq.len(), 1);
    }

    #[test]
    fn test_flush_all() {
        let mut lq = LoadQueue::new(4);
        lq.allocate(RobTag(1), MemWidth::Word);
        lq.allocate(RobTag(2), MemWidth::Word);

        lq.flush();
        assert!(lq.is_empty());
    }

    #[test]
    fn test_circular_wraparound() {
        let mut lq = LoadQueue::new(2);
        for i in 1..=10 {
            let tag = RobTag(i);
            lq.allocate(tag, MemWidth::Word);
            lq.fill_address(tag, 0x1000, 0x8000_0000);
            lq.fill_data(tag, i as u64);
            lq.deallocate(tag);
        }
    }
}
