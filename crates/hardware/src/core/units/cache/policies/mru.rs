//! Most Recently Used (MRU) Replacement Policy.
//!
//! This policy evicts the cache line that was accessed most recently.
//! While counter-intuitive for standard workloads, MRU is optimal for
//! cyclic access patterns (loops) where the dataset is larger than the cache.
//! In such cases, the most recently used item is the least likely to be
//! needed again in the immediate future.

use super::ReplacementPolicy;

/// MRU Policy state.
pub struct MruPolicy {
    /// A vector of usage stacks (one per set).
    /// Index 0 is the MRU position (victim), last index is LRU.
    usage: Vec<Vec<usize>>,
}

impl MruPolicy {
    /// Creates a new MRU policy instance.
    ///
    /// # Arguments
    ///
    /// * `sets` - The number of sets in the cache.
    /// * `ways` - The associativity (number of ways) of the cache.
    pub fn new(sets: usize, ways: usize) -> Self {
        let mut usage = Vec::with_capacity(sets);
        for _ in 0..sets {
            usage.push((0..ways).collect());
        }
        Self { usage }
    }
}

impl ReplacementPolicy for MruPolicy {
    /// Updates the policy state on access.
    ///
    /// Moves the accessed `way` to the front of the usage stack (MRU position).
    fn update(&mut self, set: usize, way: usize) {
        let stack = &mut self.usage[set];
        if let Some(pos) = stack.iter().position(|&x| x == way) {
            stack.remove(pos);
        }
        stack.insert(0, way);
    }

    /// Identifies the victim way to evict.
    ///
    /// Returns the way at the top of the usage stack (the Most Recently Used).
    fn get_victim(&mut self, set: usize) -> usize {
        *self.usage[set].first().unwrap()
    }
}
