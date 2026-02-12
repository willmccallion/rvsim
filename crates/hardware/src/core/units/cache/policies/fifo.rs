//! First-In, First-Out (FIFO) Replacement Policy.
//!
//! This policy evicts the oldest cache line in a set, regardless of how recently
//! it was accessed. It operates as a circular buffer (Round-Robin) for each set.
//! When a replacement is needed, the pointer advances to the next way.
//!
//! # Performance
//!
//! - **Time Complexity:**
//!   - `update()`: O(1)
//!   - `get_victim()`: O(1)
//! - **Space Complexity:** O(S) where S is the number of sets
//! - **Hardware Cost:** Minimal - single counter per set
//! - **Best Case:** Streaming accesses where all lines have equal importance
//! - **Worst Case:** Workloads with strong temporal locality (may evict frequently-used lines)

use super::ReplacementPolicy;

/// FIFO Policy state.
pub struct FifoPolicy {
    /// Tracks the next way to be evicted for each set.
    next_way: Vec<usize>,
    /// Number of ways in the cache.
    ways: usize,
}

impl FifoPolicy {
    /// Creates a new FIFO policy instance.
    ///
    /// # Arguments
    ///
    /// * `sets` - The number of sets in the cache.
    /// * `ways` - The associativity (number of ways) of the cache.
    pub fn new(sets: usize, ways: usize) -> Self {
        Self {
            next_way: vec![0; sets],
            ways,
        }
    }
}

impl ReplacementPolicy for FifoPolicy {
    /// Updates the policy state.
    ///
    /// For FIFO, if the accessed way matches the current eviction pointer,
    /// the pointer is advanced. This ensures the "first-in" order is maintained
    /// as lines are filled.
    fn update(&mut self, set: usize, way: usize) {
        if self.next_way[set] == way {
            self.next_way[set] = (self.next_way[set] + 1) % self.ways;
        }
    }

    /// Identifies the victim way to evict.
    ///
    /// Returns the current round-robin pointer for the specified set.
    fn get_victim(&mut self, set: usize) -> usize {
        self.next_way[set]
    }
}
