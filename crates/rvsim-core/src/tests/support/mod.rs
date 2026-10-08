//! What the tests share: instruction and latch builders, the simulation
//! harness, memory and interrupt mocks, multi-hart setup, and probes.

pub mod builder;
pub mod harness;
pub mod infrastructure_tests;
pub mod mocks;
pub mod multihart;
pub mod probe;

/// `n` as the non-zero count a cache's resources take.
pub fn count(n: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(n).expect("a test count is non-zero")
}
