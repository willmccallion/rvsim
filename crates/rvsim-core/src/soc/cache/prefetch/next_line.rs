//! Next-Line Prefetcher.
//!
//! A simple spatial prefetcher that fetches the next sequential cache line(s)
//! whenever a cache access occurs. This exploits the spatial locality common
//! in instruction streams and sequential data arrays.

use super::Prefetcher;
use crate::common::VirtAddr;
use std::num::NonZeroUsize;

/// Next-Line Prefetcher state.
#[derive(Debug)]
pub struct NextLinePrefetcher {
    /// Size of a cache line in bytes.
    line_bytes: u64,
    /// Number of subsequent lines to prefetch (prefetch degree).
    degree: usize,
}

impl NextLinePrefetcher {
    /// A next-line prefetcher fetching `degree` lines ahead.
    pub const fn new(line_bytes: usize, degree: NonZeroUsize) -> Self {
        Self { line_bytes: line_bytes as u64, degree: degree.get() }
    }
}

impl Prefetcher for NextLinePrefetcher {
    /// Prefetches the `degree` lines after the accessed one.
    fn observe(&mut self, addr: u64, _pc: Option<VirtAddr>, _hit: bool) -> Vec<u64> {
        let mut prefetches = Vec::new();

        for k in 1..=self.degree {
            let offset = self.line_bytes * k as u64;
            let target = (addr & !(self.line_bytes - 1)) + offset;
            prefetches.push(target);
        }
        prefetches
    }
}
