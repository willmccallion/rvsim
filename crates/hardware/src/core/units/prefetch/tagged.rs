//! Tagged Prefetcher.
//!
//! A smart prefetcher that attempts to reduce cache pollution by only
//! prefetching on demand misses or when a previously prefetched line is used.
//!
//! It maintains a history of addresses it has prefetched.
//! * If a **Miss** occurs: It assumes a new stream and prefetches the next line.
//! * If a **Hit** occurs: It checks if the hit address was previously prefetched.
//!   If yes (a "useful" prefetch), it extends the stream by prefetching the next line.
//!   If no (a standard hit), it stays idle to save bandwidth.

use super::Prefetcher;

/// Tagged Prefetcher state.
pub struct TaggedPrefetcher {
    /// Size of a cache line in bytes.
    line_bytes: u64,
    /// Number of lines to prefetch ahead.
    degree: usize,
    /// A small filter to track addresses issued by this prefetcher.
    /// Simulates the "Tag" bit usually stored in the cache line metadata.
    prefetched_filter: Vec<u64>,
    /// Mask for indexing the filter.
    filter_mask: usize,
}

impl TaggedPrefetcher {
    /// Creates a new Tagged prefetcher.
    ///
    /// # Arguments
    ///
    /// * `line_bytes` - The size of a cache line in bytes.
    /// * `degree` - The number of lines to prefetch ahead.
    pub fn new(line_bytes: usize, degree: usize) -> Self {
        let filter_size = 64;

        Self {
            line_bytes: line_bytes as u64,
            degree: if degree == 0 { 1 } else { degree },
            prefetched_filter: vec![0; filter_size],
            filter_mask: filter_size - 1,
        }
    }

    /// Checks if an address was recently prefetched.
    fn was_prefetched(&self, addr: u64) -> bool {
        let idx = ((addr >> 6) as usize) & self.filter_mask;
        self.prefetched_filter[idx] == addr
    }

    /// Marks an address as prefetched.
    fn mark_prefetched(&mut self, addr: u64) {
        let idx = ((addr >> 6) as usize) & self.filter_mask;
        self.prefetched_filter[idx] = addr;
    }
}

impl Prefetcher for TaggedPrefetcher {
    /// Observes a memory access and generates prefetch candidates.
    ///
    /// Uses the `hit` status to determine if the stream should be extended.
    ///
    /// # Arguments
    ///
    /// * `addr` - The memory address being accessed.
    /// * `hit` - Whether the access was a cache hit.
    ///
    /// # Returns
    ///
    /// A vector of addresses to prefetch.
    fn observe(&mut self, addr: u64, hit: bool) -> Vec<u64> {
        let mut prefetches = Vec::new();
        let aligned_addr = addr & !(self.line_bytes - 1);

        if !hit || self.was_prefetched(aligned_addr) {
            for k in 1..=self.degree {
                let offset = self.line_bytes * k as u64;
                let target = aligned_addr + offset;

                prefetches.push(target);
                self.mark_prefetched(target);
            }
        }

        prefetches
    }
}
