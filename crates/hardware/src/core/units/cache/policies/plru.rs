//! Pseudo-LRU (PLRU) Replacement Policy.
//!
//! PLRU approximates the Least Recently Used algorithm using a binary tree of bits.
//! It requires significantly less storage than full LRU (N-1 bits for N ways).
//!
//! Each bit in the tree represents a decision node pointing away from the most
//! recently used subtree. To find a victim, the tree is traversed following the
//! arrows (bits) to a leaf node.
//!
//! # Performance
//!
//! - **Time Complexity:**
//!   - `update()`: O(1)
//!   - `get_victim()`: O(1)
//! - **Space Complexity:** O(S × W) bits where S is sets, W is ways (much less than LRU)
//! - **Hardware Cost:** Low - simple bit operations
//! - **Best Case:** Similar to LRU for most access patterns
//! - **Worst Case:** Pathological cases can cause premature eviction of useful lines

use super::ReplacementPolicy;

/// PLRU Policy state.
pub struct PlruPolicy {
    /// Bitmask representing the tree state for each set.
    usage: Vec<u64>,
    /// Number of ways in the cache.
    ways: usize,
}

impl PlruPolicy {
    /// Creates a new PLRU policy instance.
    ///
    /// # Arguments
    ///
    /// * `sets` - The number of sets in the cache.
    /// * `ways` - The associativity (number of ways) of the cache.
    pub fn new(sets: usize, ways: usize) -> Self {
        Self {
            usage: vec![0; sets],
            ways,
        }
    }
}

impl ReplacementPolicy for PlruPolicy {
    /// Updates the tree bits on access.
    ///
    /// Sets the bits along the path to the accessed way to point away from it,
    /// protecting it from immediate eviction.
    fn update(&mut self, set: usize, way: usize) {
        let mask = 1 << way;
        self.usage[set] |= mask;

        let all_ones = (1 << self.ways) - 1;
        if (self.usage[set] & all_ones) == all_ones {
            self.usage[set] = mask;
        }
    }

    /// Identifies the victim way to evict.
    ///
    /// Traverses the tree bits to find the pseudo-least-recently-used way.
    fn get_victim(&mut self, set: usize) -> usize {
        for i in 0..self.ways {
            if (self.usage[set] >> i) & 1 == 0 {
                return i;
            }
        }
        0
    }
}
