//! First-In, First-Out (FIFO) Replacement Policy.
//!
//! This policy evicts the line installed longest ago in a set, however
//! recently it was accessed: hits do not change the order, only fills do.
//! A way refilled out of turn, as after an invalidation, becomes the
//! newest line of its set.
//!
//! # Performance
//!
//! - **Time Complexity:**
//!   - `update()`, `insert()`: O(1)
//!   - `get_victim()`: O(W) where W is the associativity
//! - **Space Complexity:** O(S × W) where S is the number of sets
//! - **Hardware Cost:** An insertion order per set
//! - **Best Case:** Streaming accesses where all lines have equal importance
//! - **Worst Case:** Workloads with strong temporal locality (may evict frequently-used lines)

use super::{ReplacementPolicy, SetIdx, WayIdx};

/// FIFO Policy state.
#[derive(Debug)]
pub struct FifoPolicy {
    /// When each way's line was installed, as `inserted_at[set * ways + way]`;
    /// 0 for a way never filled.
    inserted_at: Vec<u64>,
    /// The stamp the next installed line gets.
    next_stamp: u64,
    /// Number of ways in the cache.
    ways: usize,
}

impl FifoPolicy {
    /// A FIFO policy for a cache of `sets` sets of `ways` ways.
    pub fn new(sets: usize, ways: usize) -> Self {
        Self { inserted_at: vec![0; sets * ways], next_stamp: 1, ways }
    }
}

impl ReplacementPolicy for FifoPolicy {
    /// A hit leaves the insertion order as it is.
    fn update(&mut self, _set: SetIdx, _way: WayIdx) {}

    /// Makes `way`'s line the newest of its set.
    fn insert(&mut self, set: SetIdx, way: WayIdx) {
        let (set, way) = (set.index(), way.index());
        self.inserted_at[set * self.ways + way] = self.next_stamp;
        self.next_stamp += 1;
    }

    /// Identifies the victim way to evict: the one installed longest ago,
    /// the lowest-numbered among ways never filled.
    fn get_victim(&mut self, set: SetIdx) -> WayIdx {
        let set = set.index();
        let stamps = &self.inserted_at[set * self.ways..(set + 1) * self.ways];
        WayIdx::new((0..self.ways).min_by_key(|&way| stamps[way]).unwrap_or(0))
    }
}
