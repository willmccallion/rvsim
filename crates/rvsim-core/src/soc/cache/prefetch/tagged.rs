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
use crate::common::VirtAddr;
use std::num::NonZeroUsize;

/// Lines the filter remembers having prefetched.
const FILTER_ENTRIES: usize = 64;

/// Tagged Prefetcher state.
#[derive(Debug)]
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
    /// A tagged prefetcher fetching `degree` lines ahead.
    pub fn new(line_bytes: usize, degree: NonZeroUsize) -> Self {
        Self {
            line_bytes: line_bytes as u64,
            degree: degree.get(),
            prefetched_filter: vec![0; FILTER_ENTRIES],
            filter_mask: FILTER_ENTRIES - 1,
        }
    }

    /// The filter entry for the line at `addr`.
    const fn filter_slot(&self, addr: u64) -> usize {
        (addr / self.line_bytes) as usize & self.filter_mask
    }

    /// Checks if an address was recently prefetched.
    fn was_prefetched(&self, addr: u64) -> bool {
        self.prefetched_filter[self.filter_slot(addr)] == addr
    }

    /// Marks an address as prefetched.
    fn mark_prefetched(&mut self, addr: u64) {
        let slot = self.filter_slot(addr);
        self.prefetched_filter[slot] = addr;
    }
}

impl Prefetcher for TaggedPrefetcher {
    /// Extends the stream on a miss, or on a hit to a line it prefetched.
    fn observe(&mut self, addr: u64, _pc: Option<VirtAddr>, hit: bool) -> Vec<u64> {
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
