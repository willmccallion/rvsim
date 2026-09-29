//! Least Recently Used (LRU) Replacement Policy.
//!
//! This policy evicts the cache line that has not been accessed for the longest time.
//! It maintains a usage stack for each set. When a line is accessed, it is moved
//! to the top (Most Recently Used position). The bottom of the stack represents
//! the Least Recently Used line.
//!
//! # Performance
//!
//! - **Time Complexity:**
//!   - `update()`: O(W) where W is the number of ways (associativity)
//!   - `get_victim()`: O(1)
//! - **Space Complexity:** O(S × W) where S is the number of sets
//! - **Hardware Cost:** High - requires priority encoding and shifting
//! - **Best Case:** Sequential/streaming accesses with good temporal locality
//! - **Worst Case:** Scanning patterns larger than cache capacity (thrashing)

use super::ReplacementPolicy;
use super::recency::RecencyStacks;

/// LRU Policy state.
#[derive(Debug)]
pub struct LruPolicy {
    usage: RecencyStacks,
}

impl LruPolicy {
    /// Creates a new LRU policy instance.
    ///
    /// # Arguments
    ///
    /// * `sets` - The number of sets in the cache.
    /// * `ways` - The associativity (number of ways) of the cache.
    pub fn new(sets: usize, ways: usize) -> Self {
        Self { usage: RecencyStacks::new(sets, ways) }
    }
}

impl ReplacementPolicy for LruPolicy {
    /// Updates the policy state on access.
    ///
    /// Moves the accessed `way` to the front of the usage stack (MRU position),
    /// shifting other elements down.
    fn update(&mut self, set: usize, way: usize) {
        self.usage.touch(set, way);
    }

    /// Identifies the victim way to evict.
    ///
    /// Returns the way at the bottom of the usage stack (LRU position).
    fn get_victim(&mut self, set: usize) -> usize {
        self.usage.least_recent(set)
    }
}
