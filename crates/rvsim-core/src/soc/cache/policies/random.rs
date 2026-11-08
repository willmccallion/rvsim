//! Random Replacement Policy.
//!
//! This policy evicts a random cache line from the set. It uses a simple
//! Linear Feedback Shift Register (LFSR) to generate pseudo-random numbers,
//! avoiding the overhead of a complex RNG.

use super::{ReplacementPolicy, SetIdx, WayIdx};

/// Random Policy state.
#[derive(Debug)]
pub struct RandomPolicy {
    /// Number of ways in the cache.
    ways: usize,
    /// Internal state for the pseudo-random number generator.
    state: u64,
}

impl RandomPolicy {
    /// A random policy for a cache of `ways` ways.
    pub const fn new(ways: usize) -> Self {
        Self { ways, state: 123456789 }
    }
}

impl ReplacementPolicy for RandomPolicy {
    /// Updates the policy state.
    ///
    /// For Random replacement, access patterns do not affect the state,
    /// so this is a no-op.
    fn update(&mut self, _set: SetIdx, _way: WayIdx) {}

    /// Identifies the victim way to evict.
    ///
    /// Generates a pseudo-random number and maps it to a valid way index.
    fn get_victim(&mut self, _set: SetIdx) -> WayIdx {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        WayIdx::new((x as usize) % self.ways)
    }
}
