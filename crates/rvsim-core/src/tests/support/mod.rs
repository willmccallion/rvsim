//! What the tests share: instruction and latch builders, the simulation
//! harness, a memory mock, multi-hart setup, probes and the registers by
//! name.

use crate::arch::pmp::Pmp;
use crate::arch::pmp::PmpIdx;

pub mod builder;
pub mod harness;
pub mod mocks;
pub mod multihart;
pub mod probe;
pub mod regs;

/// PMP entry `n`.
pub fn pmp_entry(n: usize) -> PmpIdx {
    PmpIdx::new(n).expect("a test's PMP entry exists")
}

/// Opens `pmp` to every privilege: entry 0 grants RWX up to the top of
/// the address space, as a firmware would before dropping to S-mode.
pub fn open_pmp(pmp: &mut Pmp) {
    const TOR_RWX: u8 = 0b0000_1111;
    pmp.set_addr(pmp_entry(0), u64::MAX >> 10);
    pmp.set_cfg(pmp_entry(0), TOR_RWX);
}

/// The seed every property test starts from, so a run is repeatable.
const PROPTEST_SEED: u64 = 0x7276_7369_6d00_0001;

/// A property test of `cases` cases from the fixed seed.
pub fn seeded_cases(cases: u32) -> proptest::test_runner::Config {
    proptest::test_runner::Config {
        cases,
        rng_seed: proptest::test_runner::RngSeed::Fixed(PROPTEST_SEED),
        ..proptest::test_runner::Config::default()
    }
}

/// `n` as the non-zero count a cache's resources take.
pub fn count(n: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(n).expect("a test count is non-zero")
}
