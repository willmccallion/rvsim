//! Load Queue stub — placeholder for Phase 4 (memory ordering).
//!
//! The full implementation (violation detection, replay) is done in Phase 4.
//! This stub satisfies the `pub mod load_queue;` declaration so Phase 1 compiles.

use crate::core::pipeline::rob::RobTag;
use crate::core::pipeline::signals::MemWidth;

/// A single load queue entry (Phase 4 — not yet used).
#[allow(dead_code)]
pub struct LoadQueueEntry {
    pub rob_tag: RobTag,
    pub paddr: Option<u64>,
    pub width: MemWidth,
    pub executed: bool,
    pub valid: bool,
}

/// Load queue for in-flight load tracking and memory ordering violation detection.
///
/// Phase 1 stub — all operations are no-ops. Phase 4 provides the full implementation.
pub struct LoadQueue {
    capacity: usize,
}

impl LoadQueue {
    /// Create a new (empty) load queue with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self { capacity }
    }

    /// How many slots are free.
    pub fn free_slots(&self) -> usize {
        self.capacity
    }

    /// Number of in-flight loads.
    pub fn len(&self) -> usize {
        0
    }

    /// Whether the load queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Flush all entries (trap / full pipeline flush).
    pub fn flush(&mut self) {}

    /// Flush entries newer than `keep_tag` (misprediction recovery).
    pub fn flush_after(&mut self, _keep_tag: RobTag) {}
}
