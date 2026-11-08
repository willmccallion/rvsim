//! Most Recently Used (MRU) Replacement Policy.
//!
//! This policy evicts the cache line that was accessed most recently.
//! While counter-intuitive for standard workloads, MRU is optimal for
//! cyclic access patterns (loops) where the dataset is larger than the cache.
//! In such cases, the most recently used item is the least likely to be
//! needed again in the immediate future.

use super::recency::RecencyStacks;
use super::{ReplacementPolicy, SetIdx, WayIdx};

/// MRU Policy state.
#[derive(Debug)]
pub struct MruPolicy {
    usage: RecencyStacks,
}

impl MruPolicy {
    /// A MRU policy for a cache of `sets` sets of `ways` ways.
    pub fn new(sets: usize, ways: usize) -> Self {
        Self { usage: RecencyStacks::new(sets, ways) }
    }
}

impl ReplacementPolicy for MruPolicy {
    /// Updates the policy state on access.
    ///
    /// Moves the accessed `way` to the front of the usage stack (MRU position).
    fn update(&mut self, set: SetIdx, way: WayIdx) {
        let (set, way) = (set.index(), way.index());
        self.usage.touch(set, way);
    }

    /// Identifies the victim way to evict.
    ///
    /// Returns the way at the top of the usage stack (the Most Recently Used).
    fn get_victim(&mut self, set: SetIdx) -> WayIdx {
        let set = set.index();
        WayIdx::new(self.usage.most_recent(set))
    }
}
