//! Hardware Prefetcher implementations.
//!
//! This module contains the interface and implementations for various
//! hardware prefetchers used to hide memory latency.

/// Next-line prefetcher (prefetches sequential cache lines).
pub mod next_line;

/// Stream prefetcher (detects ascending/descending access streams).
pub mod stream;

/// Stride prefetcher (detects constant-stride access patterns).
pub mod stride;

/// Tagged prefetcher (prefetches on demand misses and prefetch hits).
pub mod tagged;

pub use self::next_line::NextLinePrefetcher;
pub use self::stream::StreamPrefetcher;
pub use self::stride::StridePrefetcher;
pub use self::tagged::TaggedPrefetcher;

/// Trait for cache prefetcher implementations.
///
/// Prefetchers observe memory access patterns and generate prefetch
/// requests to reduce cache miss penalties.
pub trait Prefetcher: Send + Sync {
    /// Observes a memory access and generates prefetch addresses.
    ///
    /// Called by the cache on each access to allow the prefetcher to
    /// learn access patterns and generate prefetch requests.
    ///
    /// # Arguments
    ///
    /// * `addr` - The address that was accessed
    /// * `hit` - Whether the access was a cache hit
    ///
    /// # Returns
    ///
    /// A vector of addresses to prefetch. Empty if no prefetches are needed.
    fn observe(&mut self, addr: u64, hit: bool) -> Vec<u64>;
}

/// Shared prefetch filter to deduplicate prefetch requests across cache levels.
///
/// Uses a small hash table (bloom-filter-like) to track recently issued prefetch
/// addresses. Before a prefetcher issues a request, it checks the filter to avoid
/// redundant prefetches for addresses already pending in an MSHR or recently prefetched.
///
/// The filter is shared across L1 and L2 prefetchers to prevent them from issuing
/// duplicate requests for the same address.
#[derive(Debug)]
pub struct PrefetchFilter {
    /// Hash table of recently-prefetched cache-line-aligned addresses.
    table: Vec<u64>,
    /// Mask for indexing into the table.
    mask: usize,
    /// Cache line size for address alignment.
    line_bytes: u64,
}

impl PrefetchFilter {
    /// Creates a new prefetch filter with the given number of entries.
    ///
    /// `size` is rounded up to the next power of two. A size of 0 disables the filter.
    pub fn new(size: usize, line_bytes: usize) -> Self {
        let safe_line = if line_bytes == 0 { 64 } else { line_bytes };
        if size == 0 {
            return Self { table: Vec::new(), mask: 0, line_bytes: safe_line as u64 };
        }
        let actual_size = size.next_power_of_two();
        Self {
            table: vec![u64::MAX; actual_size],
            mask: actual_size - 1,
            line_bytes: safe_line as u64,
        }
    }

    /// Returns true if the filter is disabled (0 entries).
    #[inline]
    pub const fn is_disabled(&self) -> bool {
        self.table.is_empty()
    }

    /// Checks if the address was recently prefetched (is in the filter).
    ///
    /// Returns true if the address should be skipped (already pending/prefetched).
    #[inline]
    pub fn contains(&self, addr: u64) -> bool {
        if self.table.is_empty() {
            return false;
        }
        let line_addr = addr & !(self.line_bytes - 1);
        let idx = (line_addr as usize >> 6) & self.mask;
        self.table[idx] == line_addr
    }

    /// Records that a prefetch was issued for the given address.
    #[inline]
    pub fn insert(&mut self, addr: u64) {
        if self.table.is_empty() {
            return;
        }
        let line_addr = addr & !(self.line_bytes - 1);
        let idx = (line_addr as usize >> 6) & self.mask;
        self.table[idx] = line_addr;
    }

    /// Removes an address from the filter (e.g., when the prefetch completes
    /// and the line is installed in the cache).
    #[inline]
    pub fn remove(&mut self, addr: u64) {
        if self.table.is_empty() {
            return;
        }
        let line_addr = addr & !(self.line_bytes - 1);
        let idx = (line_addr as usize >> 6) & self.mask;
        if self.table[idx] == line_addr {
            self.table[idx] = u64::MAX;
        }
    }

    /// Filters a list of prefetch addresses, removing any that are already
    /// in the filter or in the given cache. Inserts surviving addresses into
    /// the filter.
    ///
    /// Returns the filtered list of addresses that should actually be prefetched.
    pub fn filter_and_record(&mut self, addrs: Vec<u64>, dedup_count: &mut u64) -> Vec<u64> {
        if self.table.is_empty() {
            return addrs;
        }
        let mut result = Vec::with_capacity(addrs.len());
        for addr in addrs {
            if self.contains(addr) {
                *dedup_count += 1;
            } else {
                self.insert(addr);
                result.push(addr);
            }
        }
        result
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    #[test]
    fn test_disabled_filter() {
        let filter = PrefetchFilter::new(0, 64);
        assert!(filter.is_disabled());
        assert!(!filter.contains(0x1000));
    }

    #[test]
    fn test_insert_and_contains() {
        let mut filter = PrefetchFilter::new(64, 64);
        assert!(!filter.contains(0x1000));
        filter.insert(0x1000);
        assert!(filter.contains(0x1000));
        // Same cache line, different offset
        assert!(filter.contains(0x1020));
    }

    #[test]
    fn test_remove() {
        let mut filter = PrefetchFilter::new(64, 64);
        filter.insert(0x1000);
        assert!(filter.contains(0x1000));
        filter.remove(0x1000);
        assert!(!filter.contains(0x1000));
    }

    #[test]
    fn test_filter_and_record() {
        let mut filter = PrefetchFilter::new(64, 64);
        filter.insert(0x1000); // Already known
        let mut dedup = 0;
        let result = filter.filter_and_record(vec![0x1000, 0x1040, 0x1080], &mut dedup);
        assert_eq!(result.len(), 2); // 0x1040 and 0x1080
        assert_eq!(dedup, 1); // 0x1000 was deduped
        assert!(filter.contains(0x1040));
        assert!(filter.contains(0x1080));
    }
}
