//! # Register Indexing Tests
//!
//! This module provides comprehensive unit tests for the `RegisterFile` structure,
//! ensuring that both General Purpose Registers (GPRs) and Floating-Point Registers (FPRs)
//! behave according to the RISC-V architectural specifications.
//!
//! The tests cover initialization, read/write consistency, the invariant that `x0`
//! remains zero, and the independence of the integer and floating-point register sets.

use riscv_core::common::reg::RegisterFile;

/// Ensures that all general-purpose registers are initialized to zero upon creation.
#[test]
fn gpr_initial_values_are_zero() {
    let regs = RegisterFile::new();
    for i in 0..32 {
        assert_eq!(regs.read(i), 0, "x{} should be 0 initially", i);
    }
}

/// Verifies that a value written to a general-purpose register can be correctly read back.
#[test]
fn gpr_write_and_read() {
    let mut regs = RegisterFile::new();
    regs.write(1, 42);
    assert_eq!(regs.read(1), 42);
}

/// Ensures that register `x0` remains zero regardless of any values written to it,
/// as per the RISC-V specification.
#[test]
fn gpr_x0_always_zero() {
    let mut regs = RegisterFile::new();
    regs.write(0, 0xDEAD_BEEF);
    assert_eq!(regs.read(0), 0, "x0 must always read as 0");
}

/// Verifies that all registers (x1-x31) can hold independent values simultaneously
/// while ensuring x0 remains zero.
#[test]
fn gpr_write_all_registers() {
    let mut regs = RegisterFile::new();
    for i in 0..32 {
        regs.write(i, i as u64 * 100);
    }
    assert_eq!(regs.read(0), 0, "x0 must remain 0");
    for i in 1..32 {
        assert_eq!(regs.read(i), i as u64 * 100);
    }
}

/// Verifies that writing a new value to a register correctly overwrites the previous value.
#[test]
fn gpr_overwrite() {
    let mut regs = RegisterFile::new();
    regs.write(5, 100);
    assert_eq!(regs.read(5), 100);
    regs.write(5, 200);
    assert_eq!(regs.read(5), 200);
}

/// Verifies that registers can store the maximum possible 64-bit unsigned integer value.
#[test]
fn gpr_max_value() {
    let mut regs = RegisterFile::new();
    regs.write(31, u64::MAX);
    assert_eq!(regs.read(31), u64::MAX);
}

/// Verifies that all floating-point registers (FPRs) are initialized to zero.
#[test]
fn fpr_initial_values_are_zero() {
    let regs = RegisterFile::new();
    for i in 0..32 {
        assert_eq!(regs.read_f(i), 0, "f{} should be 0 initially", i);
    }
}

/// Verifies that a value written to a floating-point register can be read back correctly.
#[test]
fn fpr_write_and_read() {
    let mut regs = RegisterFile::new();
    #[allow(clippy::approx_constant)]
    let val = f64::to_bits(3.14);
    regs.write_f(0, val);
    assert_eq!(regs.read_f(0), val);
}

/// Ensures that `f0` behaves as a normal register and is not hardwired to zero,
/// unlike the integer register `x0`.
#[test]
fn fpr_f0_is_writable() {
    // Unlike x0, f0 is a normal register
    let mut regs = RegisterFile::new();
    #[allow(clippy::approx_constant)]
    let val = f64::to_bits(2.71828);
    regs.write_f(0, val);
    assert_eq!(regs.read_f(0), val);
}

/// Verifies that all 32 floating-point registers can store and retrieve values independently.
#[test]
fn fpr_write_all_registers() {
    let mut regs = RegisterFile::new();
    for i in 0..32 {
        regs.write_f(i, (i as u64 + 1) * 1000);
    }
    for i in 0..32 {
        assert_eq!(regs.read_f(i), (i as u64 + 1) * 1000);
    }
}

/// Ensures that NaN-boxed values (often used for `f32` in `f64` registers)
/// preserve their raw bit representation when stored in the register file.
#[test]
fn fpr_nan_boxing_bits() {
    // Storing a NaN-boxed f32 should preserve the raw bits
    let mut regs = RegisterFile::new();
    let boxed: u64 = 0xFFFF_FFFF_3FC0_0000; // NaN-boxed 1.5f32
    regs.write_f(10, boxed);
    assert_eq!(regs.read_f(10), boxed);
}

/// Verifies that the General Purpose Registers (GPR) and Floating Point Registers (FPR)
/// are independent and do not share storage.
#[test]
fn gpr_fpr_independent() {
    let mut regs = RegisterFile::new();
    regs.write(5, 0xAAAA);
    regs.write_f(5, 0xBBBB);
    assert_eq!(regs.read(5), 0xAAAA);
    assert_eq!(regs.read_f(5), 0xBBBB);
}
