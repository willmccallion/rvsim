//! Random Replacement Policy.
//!
//! This policy evicts a random cache line from the set. It uses a simple
//! Linear Feedback Shift Register (LFSR) to generate pseudo-random numbers,
//! avoiding the overhead of a complex RNG.

use super::ReplacementPolicy;

/// Random Policy state.
pub struct RandomPolicy {
    /// Number of ways in the cache.
    ways: usize,
    /// Internal state for the pseudo-random number generator.
    state: u64,
}

impl RandomPolicy {
    /// Creates a new Random policy instance.
    ///
    /// # Arguments
    ///
    /// * `sets` - The number of sets (unused in this policy but required by interface).
    /// * `ways` - The associativity (number of ways) of the cache.
    pub fn new(_sets: usize, ways: usize) -> Self {
        Self {
            ways,
            state: 123456789,
        }
    }
}

impl ReplacementPolicy for RandomPolicy {
    /// Updates the policy state.
    ///
    /// For Random replacement, access patterns do not affect the state,
    /// so this is a no-op.
    fn update(&mut self, _set: usize, _way: usize) {}

    /// Identifies the victim way to evict.
    ///
    /// Generates a pseudo-random number and maps it to a valid way index.
    fn get_victim(&mut self, _set: usize) -> usize {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        (x as usize) % self.ways
    }
}
