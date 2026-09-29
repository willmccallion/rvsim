//! Most Recently Used (MRU) Replacement Policy.
//!
//! This policy evicts the cache line that was accessed most recently.
//! While counter-intuitive for standard workloads, MRU is optimal for
//! cyclic access patterns (loops) where the dataset is larger than the cache.
//! In such cases, the most recently used item is the least likely to be
//! needed again in the immediate future.

use super::ReplacementPolicy;
use super::recency::RecencyStacks;

/// MRU Policy state.
#[derive(Debug)]
pub struct MruPolicy {
    usage: RecencyStacks,
}

impl MruPolicy {
    /// Creates a new MRU policy instance.
    ///
    /// # Arguments
    ///
    /// * `sets` - The number of sets in the cache.
    /// * `ways` - The associativity (number of ways) of the cache.
    pub fn new(sets: usize, ways: usize) -> Self {
        Self { usage: RecencyStacks::new(sets, ways) }
    }
}

impl ReplacementPolicy for MruPolicy {
    /// Updates the policy state on access.
    ///
    /// Moves the accessed `way` to the front of the usage stack (MRU position).
    fn update(&mut self, set: usize, way: usize) {
        self.usage.touch(set, way);
    }

    /// Identifies the victim way to evict.
    ///
    /// Returns the way at the top of the usage stack (the Most Recently Used).
    fn get_victim(&mut self, set: usize) -> usize {
        self.usage.most_recent(set)
    }
}
