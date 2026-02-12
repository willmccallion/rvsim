//! RISC-V Exception Flag tests.
//!
//! These tests verify that `Fpu::execute_full()` returns both the
//! result and the correct accrued exception flags (NV, DZ, OF, NX).

use riscv_core::core::pipeline::signals::AluOp;
use riscv_core::core::units::fpu::Fpu;
use riscv_core::core::units::fpu::exception_flags::FpFlags;

#[test]
fn test_exception_dz() {
    // 1.0 / 0.0 should set DZ flag
    let one = Fpu::box_f32(1.0);
    let zero = Fpu::box_f32(0.0);
    let (result, flags) = Fpu::execute_full(AluOp::FDiv, one, zero, 0, true);

    // Result should be +infinity
    let res_f32 = f32::from_bits(result as u32);
    assert!(res_f32.is_infinite(), "1.0 / 0.0 should produce infinity");
    assert!(res_f32.is_sign_positive());

    // DZ flag should be set
    assert!(flags.contains(FpFlags::DZ), "DZ flag must be set for x/0");
    assert!(
        !flags.contains(FpFlags::NV),
        "NV should NOT be set for x/0 (x != 0)"
    );
}

#[test]
fn test_exception_nv() {
    // sqrt(-1.0) should set NV flag
    let neg_one = Fpu::box_f32(-1.0);
    let (_result, flags) = Fpu::execute_full(AluOp::FSqrt, neg_one, 0, 0, true);

    assert!(
        flags.contains(FpFlags::NV),
        "NV flag must be set for sqrt(-1.0)"
    );
}

#[test]
fn test_exception_of_nx() {
    // Large value multiplication resulting in overflow to infinity
    let large = Fpu::box_f32(f32::MAX);
    let two = Fpu::box_f32(2.0);
    let (result, flags) = Fpu::execute_full(AluOp::FMul, large, two, 0, true);

    let res_f32 = f32::from_bits(result as u32);
    assert!(res_f32.is_infinite(), "MAX * 2 should overflow to infinity");

    assert!(
        flags.contains(FpFlags::OF),
        "OF flag must be set on overflow"
    );
    assert!(
        flags.contains(FpFlags::NX),
        "NX flag must be set on overflow (inexact)"
    );
}

#[test]
fn test_exception_nv_zero_div_zero() {
    // 0.0 / 0.0 should set NV (invalid), not DZ
    let zero = Fpu::box_f32(0.0);
    let (_result, flags) = Fpu::execute_full(AluOp::FDiv, zero, zero, 0, true);

    assert!(
        flags.contains(FpFlags::NV),
        "0/0 should set NV (invalid operation)"
    );
    assert!(!flags.contains(FpFlags::DZ), "0/0 should NOT set DZ");
}

#[test]
fn test_no_exception_normal_add() {
    let a = Fpu::box_f32(1.0);
    let b = Fpu::box_f32(2.0);
    let (_result, flags) = Fpu::execute_full(AluOp::FAdd, a, b, 0, true);

    assert!(flags.is_empty(), "Normal addition should raise no flags");
}

#[test]
fn test_exception_snan_input_sets_nv() {
    // sNaN input to addition should set NV
    let snan_bits = 0x7f800001u32; // sNaN (quiet bit=0, payload!=0)
    let snan = Fpu::box_f32(f32::from_bits(snan_bits));
    let one = Fpu::box_f32(1.0);
    let (_result, flags) = Fpu::execute_full(AluOp::FAdd, snan, one, 0, true);

    assert!(flags.contains(FpFlags::NV), "sNaN input must raise NV");
}

#[test]
fn test_exception_dz_f64() {
    // f64: 1.0 / 0.0 should set DZ
    let one = f64::to_bits(1.0);
    let zero = f64::to_bits(0.0);
    let (result, flags) = Fpu::execute_full(AluOp::FDiv, one, zero, 0, false);

    let res_f64 = f64::from_bits(result);
    assert!(res_f64.is_infinite());
    assert!(flags.contains(FpFlags::DZ));
}

#[test]
fn test_exception_nv_sqrt_neg_f64() {
    let neg_one = f64::to_bits(-1.0);
    let (_result, flags) = Fpu::execute_full(AluOp::FSqrt, neg_one, 0, 0, false);
    assert!(flags.contains(FpFlags::NV));
}

#[test]
fn test_fpflags_bitor() {
    let combined = FpFlags::NV | FpFlags::DZ;
    assert!(combined.contains(FpFlags::NV));
    assert!(combined.contains(FpFlags::DZ));
    assert!(!combined.contains(FpFlags::OF));
}

#[test]
fn test_fpflags_bits() {
    assert_eq!(FpFlags::NV.bits(), 0b10000);
    assert_eq!(FpFlags::DZ.bits(), 0b01000);
    assert_eq!(FpFlags::OF.bits(), 0b00100);
    assert_eq!(FpFlags::UF.bits(), 0b00010);
    assert_eq!(FpFlags::NX.bits(), 0b00001);
    assert_eq!(FpFlags::NONE.bits(), 0);
}
