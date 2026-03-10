//! Write Combining Buffer (WCB) for store coalescing.
//!
//! Sequential stores to adjacent bytes within the same cache line are coalesced
//! into a single WCB entry before being drained to the L1D cache. This reduces
//! L1D write port pressure for sequential store patterns (e.g., memcpy, struct init).
//!
//! The WCB sits between the store buffer drain path and L1D:
//! ```text
//! Store Buffer → drain_one() → WCB → drain to L1D → write to RAM
//! ```
//!
//! An entry is drained when:
//! 1. It is full (all bytes in the cache line have been written).
//! 2. It is evicted (all entries are occupied and a new line needs an entry).
//! 3. A FENCE instruction forces all entries to drain.
//! 4. A load hits the same cache line (not modeled here — loads bypass the WCB
//!    via the store buffer forwarding path which is upstream).

/// A single WCB entry covering one cache line.
#[derive(Clone, Debug)]
struct WcbEntry {
    /// Cache-line-aligned physical address of this entry.
    line_addr: u64,
    /// Byte-level valid mask. Bit `i` is set if byte `i` has been written.
    valid_mask: u64,
    /// Data buffer (up to 64 bytes, cache-line sized).
    data: [u8; 64],
    /// Whether this entry is occupied.
    active: bool,
    /// LRU counter (lower = older = next eviction candidate).
    lru_counter: u64,
}

impl Default for WcbEntry {
    fn default() -> Self {
        Self { line_addr: 0, valid_mask: 0, data: [0; 64], active: false, lru_counter: 0 }
    }
}

/// A pending drain request returned by the WCB to the caller.
#[derive(Clone, Debug)]
pub struct WcbDrain {
    /// Cache-line-aligned physical address.
    pub line_addr: u64,
}

/// Write Combining Buffer with configurable entry count.
#[derive(Debug)]
pub struct WriteCombiningBuffer {
    entries: Vec<WcbEntry>,
    line_bytes: usize,
    /// Monotonically increasing counter for LRU tracking.
    access_counter: u64,
}

impl WriteCombiningBuffer {
    /// Creates a new WCB with the given number of entries and cache line size.
    ///
    /// A capacity of 0 disables the WCB (all stores pass through directly).
    pub fn new(capacity: usize, line_bytes: usize) -> Self {
        let safe_line = if line_bytes == 0 { 64 } else { line_bytes };
        Self {
            entries: vec![WcbEntry::default(); capacity],
            line_bytes: safe_line,
            access_counter: 0,
        }
    }

    /// Returns true if the WCB is disabled (0 entries).
    #[inline]
    pub const fn is_disabled(&self) -> bool {
        self.entries.is_empty()
    }

    /// Merges a store into the WCB.
    ///
    /// If the store's cache line already has an active WCB entry, the bytes are
    /// merged into it (coalesced). Otherwise, a new entry is allocated — evicting
    /// the LRU entry if necessary.
    ///
    /// Returns `Some(WcbDrain)` if an entry was evicted to make room, meaning the
    /// caller should drain that entry through the cache hierarchy. Returns `None`
    /// if the store was absorbed without eviction.
    pub fn merge_store(
        &mut self,
        paddr: crate::common::PhysAddr,
        data: u64,
        width_bytes: usize,
    ) -> Option<WcbDrain> {
        if self.entries.is_empty() {
            return None;
        }

        let raw = paddr.val();
        let line_mask = !(self.line_bytes as u64 - 1);
        let line_addr = raw & line_mask;
        let offset = (raw - line_addr) as usize;

        self.access_counter += 1;

        // Try to find an existing entry for this cache line
        for entry in &mut self.entries {
            if entry.active && entry.line_addr == line_addr {
                // Coalesce: merge bytes into existing entry
                Self::write_bytes(entry, offset, data, width_bytes);
                entry.lru_counter = self.access_counter;
                return None;
            }
        }

        // No existing entry — try to find a free slot
        for entry in &mut self.entries {
            if !entry.active {
                entry.active = true;
                entry.line_addr = line_addr;
                entry.valid_mask = 0;
                entry.data = [0; 64];
                Self::write_bytes(entry, offset, data, width_bytes);
                entry.lru_counter = self.access_counter;
                return None;
            }
        }

        // All entries occupied — evict LRU
        let lru_idx = self.find_lru();
        let evicted_addr = self.entries[lru_idx].line_addr;

        // Reset the entry for the new line
        self.entries[lru_idx].line_addr = line_addr;
        self.entries[lru_idx].valid_mask = 0;
        self.entries[lru_idx].data = [0; 64];
        Self::write_bytes(&mut self.entries[lru_idx], offset, data, width_bytes);
        self.entries[lru_idx].lru_counter = self.access_counter;

        Some(WcbDrain { line_addr: evicted_addr })
    }

    /// Flushes all active WCB entries, returning their addresses.
    ///
    /// Called on FENCE instructions or pipeline flush to ensure all pending
    /// writes become visible.
    pub fn flush_all(&mut self) -> Vec<WcbDrain> {
        let mut drains = Vec::new();
        for entry in &mut self.entries {
            if entry.active {
                drains.push(WcbDrain { line_addr: entry.line_addr });
                entry.active = false;
            }
        }
        drains
    }

    /// Checks if the WCB has an active entry for the given cache line address.
    ///
    /// Used to ensure cache accesses don't miss data sitting in the WCB.
    #[inline]
    pub fn contains_line(&self, line_addr: u64) -> bool {
        self.entries.iter().any(|e| e.active && e.line_addr == line_addr)
    }

    /// Returns the number of active entries.
    #[inline]
    pub fn active_count(&self) -> usize {
        self.entries.iter().filter(|e| e.active).count()
    }

    /// Writes `width_bytes` of data into the entry at the given offset.
    fn write_bytes(entry: &mut WcbEntry, offset: usize, data: u64, width_bytes: usize) {
        let bytes = data.to_le_bytes();
        let end = (offset + width_bytes).min(64);
        for i in offset..end {
            let byte_idx = i - offset;
            if byte_idx < 8 {
                entry.data[i] = bytes[byte_idx];
                entry.valid_mask |= 1u64 << i;
            }
        }
    }

    /// Finds the index of the LRU (least recently used) entry.
    fn find_lru(&self) -> usize {
        let mut min_counter = u64::MAX;
        let mut min_idx = 0;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.active && entry.lru_counter < min_counter {
                min_counter = entry.lru_counter;
                min_idx = i;
            }
        }
        min_idx
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::PhysAddr;

    #[test]
    fn test_disabled_wcb() {
        let mut wcb = WriteCombiningBuffer::new(0, 64);
        assert!(wcb.is_disabled());
        assert!(wcb.merge_store(PhysAddr::new(0x1000), 42, 4).is_none());
    }

    #[test]
    fn test_coalesce_same_line() {
        let mut wcb = WriteCombiningBuffer::new(4, 64);
        // First store to line 0x1000..0x103F
        assert!(wcb.merge_store(PhysAddr::new(0x1000), 0xAA, 1).is_none());
        assert_eq!(wcb.active_count(), 1);
        // Second store to same line, different offset
        assert!(wcb.merge_store(PhysAddr::new(0x1008), 0xBB, 1).is_none());
        assert_eq!(wcb.active_count(), 1); // coalesced
    }

    #[test]
    fn test_different_lines_allocate_new_entries() {
        let mut wcb = WriteCombiningBuffer::new(4, 64);
        assert!(wcb.merge_store(PhysAddr::new(0x1000), 1, 4).is_none());
        assert!(wcb.merge_store(PhysAddr::new(0x1040), 2, 4).is_none());
        assert!(wcb.merge_store(PhysAddr::new(0x1080), 3, 4).is_none());
        assert_eq!(wcb.active_count(), 3);
    }

    #[test]
    fn test_eviction_on_full() {
        let mut wcb = WriteCombiningBuffer::new(2, 64);
        assert!(wcb.merge_store(PhysAddr::new(0x1000), 1, 4).is_none());
        assert!(wcb.merge_store(PhysAddr::new(0x1040), 2, 4).is_none());
        // Third line should evict the LRU (0x1000)
        let drain = wcb.merge_store(PhysAddr::new(0x1080), 3, 4);
        assert!(drain.is_some());
        assert_eq!(drain.unwrap().line_addr, 0x1000);
        assert_eq!(wcb.active_count(), 2);
    }

    #[test]
    fn test_flush_all() {
        let mut wcb = WriteCombiningBuffer::new(4, 64);
        wcb.merge_store(PhysAddr::new(0x1000), 1, 4);
        wcb.merge_store(PhysAddr::new(0x1040), 2, 4);
        let drains = wcb.flush_all();
        assert_eq!(drains.len(), 2);
        assert_eq!(wcb.active_count(), 0);
    }

    #[test]
    fn test_lru_updates_on_access() {
        let mut wcb = WriteCombiningBuffer::new(2, 64);
        // Allocate two lines
        wcb.merge_store(PhysAddr::new(0x1000), 1, 4); // counter=1
        wcb.merge_store(PhysAddr::new(0x1040), 2, 4); // counter=2
        // Touch 0x1000 again (coalesce), making it MRU (counter=3)
        wcb.merge_store(PhysAddr::new(0x1004), 3, 4);
        // Now 0x1040 (counter=2) is LRU — evict it
        let drain = wcb.merge_store(PhysAddr::new(0x1080), 4, 4);
        assert!(drain.is_some());
        assert_eq!(drain.unwrap().line_addr, 0x1040);
    }

    #[test]
    fn test_contains_line() {
        let mut wcb = WriteCombiningBuffer::new(4, 64);
        wcb.merge_store(PhysAddr::new(0x1008), 1, 4);
        assert!(wcb.contains_line(0x1000));
        assert!(!wcb.contains_line(0x1040));
    }
}
