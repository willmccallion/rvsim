//! Next-Line Prefetcher.
//!
//! A simple spatial prefetcher that fetches the next sequential cache line(s)
//! whenever a cache access occurs. This exploits the spatial locality common
//! in instruction streams and sequential data arrays.

use super::Prefetcher;

/// Next-Line Prefetcher state.
pub struct NextLinePrefetcher {
    /// Size of a cache line in bytes.
    line_bytes: u64,
    /// Number of subsequent lines to prefetch (prefetch degree).
    degree: usize,
}

impl NextLinePrefetcher {
    /// Creates a new Next-Line prefetcher.
    ///
    /// # Arguments
    ///
    /// * `line_bytes` - The size of a cache line in bytes.
    /// * `degree` - The number of lines to prefetch ahead.
    pub fn new(line_bytes: usize, degree: usize) -> Self {
        Self {
            line_bytes: line_bytes as u64,
            degree: if degree == 0 { 1 } else { degree },
        }
    }
}

impl Prefetcher for NextLinePrefetcher {
    /// Observes a memory access and generates prefetch candidates.
    ///
    /// Calculates the addresses of the next `degree` cache lines following
    /// the accessed address.
    ///
    /// # Arguments
    ///
    /// * `addr` - The memory address being accessed.
    /// * `_hit` - Whether the access was a cache hit (ignored by this prefetcher).
    ///
    /// # Returns
    ///
    /// A vector of addresses to prefetch.
    fn observe(&mut self, addr: u64, _hit: bool) -> Vec<u64> {
        let mut prefetches = Vec::new();

        for k in 1..=self.degree {
            let offset = self.line_bytes * k as u64;
            let target = (addr & !(self.line_bytes - 1)) + offset;
            prefetches.push(target);
        }
        prefetches
    }
}
