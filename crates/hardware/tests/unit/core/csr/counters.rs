//! # CSR Counters Tests
//!
//! This module contains unit tests for the RISC-V Control and Status Register (CSR) counters.
//! It verifies the behavior of performance-related counters such as `cycle`, `instret`,
//! and their machine-level equivalents `mcycle` and `minstret`.
//!
//! The tests ensure that:
//! - Counters can be incremented correctly.
//! - Counters handle maximum `u64` values.
//! - Counters wrap around correctly on overflow.

use riscv_core::core::arch::csr::Csrs;

/// Tests basic increment functionality for cycle and instruction counters.
#[test]
fn counters_increment() {
    let mut csrs = Csrs::default();
    csrs.cycle += 1;
    csrs.instret += 1;
    csrs.mcycle += 1;
    csrs.minstret += 1;

    assert_eq!(csrs.cycle, 1);
    assert_eq!(csrs.instret, 1);
    assert_eq!(csrs.mcycle, 1);
    assert_eq!(csrs.minstret, 1);
}

/// Verifies that counters can store the maximum possible 64-bit value.
#[test]
fn counters_large_values() {
    let mut csrs = Csrs::default();
    csrs.cycle = u64::MAX;
    csrs.instret = u64::MAX;
    assert_eq!(csrs.cycle, u64::MAX);
    assert_eq!(csrs.instret, u64::MAX);
}

/// Ensures that counters correctly wrap around to zero on overflow.
#[test]
fn counters_wrapping() {
    let mut csrs = Csrs::default();
    csrs.cycle = u64::MAX;
    csrs.cycle = csrs.cycle.wrapping_add(1);
    assert_eq!(csrs.cycle, 0);
}
