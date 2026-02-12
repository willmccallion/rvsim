//! # Floating-Point Register (FPR) Tests
//!
//! This module contains unit tests for the Floating-Point Register (FPR) file implementation.
//! It verifies the correct behavior of the 32 floating-point registers, including
//! initialization, basic read/write operations, and the preservation of special
//! IEEE 754 values such as NaNs, infinities, and subnormals.
//!
//! A specific focus is placed on validating NaN-boxing, a RISC-V requirement where
//! narrower floating-point values (like 32-bit `f32`) are stored in wider registers
//! (like 64-bit `f64`) by setting the upper bits to all ones.

use riscv_core::core::arch::fpr::Fpr;

/// Ensures that all floating-point registers are initialized to zero upon creation.
#[test]
fn fpr_all_registers_initially_zero() {
    let fpr = Fpr::new();
    for i in 0..32 {
        assert_eq!(fpr.read(i), 0, "f{} should be 0 initially", i);
    }
}

/// Verifies that a value written to a register can be correctly read back.
#[test]
fn fpr_write_read_basic() {
    let mut fpr = Fpr::new();
    #[allow(clippy::approx_constant)]
    let bits = f64::to_bits(3.14159);
    fpr.write(0, bits);
    assert_eq!(fpr.read(0), bits);
}

/// Confirms that each of the 32 registers can store and retrieve unique values independently.
#[test]
fn fpr_write_all_registers() {
    let mut fpr = Fpr::new();
    for i in 0..32 {
        fpr.write(i, (i as u64 + 1) * 0x1000);
    }
    for i in 0..32 {
        assert_eq!(fpr.read(i), (i as u64 + 1) * 0x1000);
    }
}

/// Validates that writing a new value to a register overwrites the existing content.
#[test]
fn fpr_overwrite() {
    let mut fpr = Fpr::new();
    fpr.write(5, f64::to_bits(1.0));
    assert_eq!(fpr.read(5), f64::to_bits(1.0));
    fpr.write(5, f64::to_bits(2.0));
    assert_eq!(fpr.read(5), f64::to_bits(2.0));
}

/// Verifies that floating-point register 0 is writable and readable.
#[test]
fn fpr_f0_writable() {
    let mut fpr = Fpr::new();
    fpr.write(0, f64::to_bits(42.0));
    assert_eq!(fpr.read(0), f64::to_bits(42.0));
}

/// Verifies that a NaN-boxed `f32` value (where the upper 32 bits are all 1s)
/// is preserved correctly when stored in a 64-bit register.
#[test]
fn fpr_nan_boxed_f32_preserved() {
    let mut fpr = Fpr::new();
    // NaN-boxed f32: upper 32 bits all 1s
    let boxed: u64 = 0xFFFF_FFFF_3F80_0000; // 1.0f32 NaN-boxed
    fpr.write(10, boxed);
    assert_eq!(fpr.read(10), boxed);
}

/// Verifies that the bit pattern for a canonical 64-bit NaN is preserved.
#[test]
fn fpr_canonical_nan_preserved() {
    let mut fpr = Fpr::new();
    // f64 canonical NaN
    let canon_nan_64 = 0x7FF8_0000_0000_0000u64;
    fpr.write(15, canon_nan_64);
    assert_eq!(fpr.read(15), canon_nan_64);
}

/// Verifies that the bit representation of negative zero is preserved.
#[test]
fn fpr_negative_zero_preserved() {
    let mut fpr = Fpr::new();
    let neg_zero = f64::to_bits(-0.0);
    fpr.write(1, neg_zero);
    assert_eq!(fpr.read(1), neg_zero);
}

/// Verifies that the bit patterns for both positive and negative infinity are preserved.
#[test]
fn fpr_infinity_preserved() {
    let mut fpr = Fpr::new();
    let pos_inf = f64::to_bits(f64::INFINITY);
    let neg_inf = f64::to_bits(f64::NEG_INFINITY);
    fpr.write(2, pos_inf);
    fpr.write(3, neg_inf);
    assert_eq!(fpr.read(2), pos_inf);
    assert_eq!(fpr.read(3), neg_inf);
}

/// Verifies that subnormal floating-point numbers are preserved correctly.
#[test]
fn fpr_subnormal_preserved() {
    let mut fpr = Fpr::new();
    let subnormal = f64::to_bits(f64::MIN_POSITIVE / 2.0);
    fpr.write(20, subnormal);
    assert_eq!(fpr.read(20), subnormal);
}

/// Verifies that the register can store and retrieve the maximum `u64` value.
#[test]
fn fpr_max_value() {
    let mut fpr = Fpr::new();
    fpr.write(31, u64::MAX);
    assert_eq!(fpr.read(31), u64::MAX);
}
