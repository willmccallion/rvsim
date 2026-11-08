//! RISC-V Exception Flag tests.
//!
//! These tests verify that `fpu::execute()` returns both the
//! result and the correct accrued exception flags (NV, DZ, OF, NX).

use crate::exec::compute::fpu;
use crate::exec::compute::fpu::float::Float;
use crate::isa::fp::{FpFlags, FpFormat, RoundingMode};
use crate::isa::op::{FmaSigns, FpArithOp, FpOp};

/// `value` NaN-boxed in a 64-bit FP register.
fn to_register(value: f32) -> u64 {
    Float::to_register(value)
}

#[test]
fn single_one_over_zero_is_infinity_raising_dz_not_nv() {
    // 1.0 / 0.0 should set DZ flag
    let one = to_register(1.0);
    let zero = to_register(0.0);
    let (result, flags) = fpu::execute(
        FpOp::Arith(FpArithOp::Div),
        [one, zero, 0],
        FpFormat::Single,
        RoundingMode::Rne,
    );

    // Result should be +infinity
    let res_f32 = f32::from_bits(result as u32);
    assert!(res_f32.is_infinite(), "1.0 / 0.0 should produce infinity");
    assert!(res_f32.is_sign_positive());

    // DZ flag should be set
    assert!(flags.contains(FpFlags::DZ), "DZ flag must be set for x/0");
    assert!(!flags.contains(FpFlags::NV), "NV should NOT be set for x/0 (x != 0)");
}

#[test]
fn single_sqrt_of_minus_one_raises_nv() {
    // sqrt(-1.0) should set NV flag
    let neg_one = to_register(-1.0);
    let (_result, flags) =
        fpu::execute(FpOp::Sqrt, [neg_one, 0, 0], FpFormat::Single, RoundingMode::Rne);

    assert!(flags.contains(FpFlags::NV), "NV flag must be set for sqrt(-1.0)");
}

#[test]
fn single_overflow_to_infinity_raises_of_and_nx() {
    // Large value multiplication resulting in overflow to infinity
    let large = to_register(f32::MAX);
    let two = to_register(2.0);
    let (result, flags) = fpu::execute(
        FpOp::Arith(FpArithOp::Mul),
        [large, two, 0],
        FpFormat::Single,
        RoundingMode::Rne,
    );

    let res_f32 = f32::from_bits(result as u32);
    assert!(res_f32.is_infinite(), "MAX * 2 should overflow to infinity");

    assert!(flags.contains(FpFlags::OF), "OF flag must be set on overflow");
    assert!(flags.contains(FpFlags::NX), "NX flag must be set on overflow (inexact)");
}

#[test]
fn single_zero_over_zero_raises_nv_not_dz() {
    // 0.0 / 0.0 should set NV (invalid), not DZ
    let zero = to_register(0.0);
    let (_result, flags) = fpu::execute(
        FpOp::Arith(FpArithOp::Div),
        [zero, zero, 0],
        FpFormat::Single,
        RoundingMode::Rne,
    );

    assert!(flags.contains(FpFlags::NV), "0/0 should set NV (invalid operation)");
    assert!(!flags.contains(FpFlags::DZ), "0/0 should NOT set DZ");
}

#[test]
fn an_exact_single_add_raises_no_flags() {
    let a = to_register(1.0);
    let b = to_register(2.0);
    let (_result, flags) =
        fpu::execute(FpOp::Arith(FpArithOp::Add), [a, b, 0], FpFormat::Single, RoundingMode::Rne);

    assert!(flags.is_empty(), "Normal addition should raise no flags");
}

#[test]
fn a_signalling_nan_operand_to_add_raises_nv() {
    // sNaN input to addition should set NV
    let snan_bits = 0x7f800001u32; // sNaN (quiet bit=0, payload!=0)
    let snan = to_register(f32::from_bits(snan_bits));
    let one = to_register(1.0);
    let (_result, flags) = fpu::execute(
        FpOp::Arith(FpArithOp::Add),
        [snan, one, 0],
        FpFormat::Single,
        RoundingMode::Rne,
    );

    assert!(flags.contains(FpFlags::NV), "sNaN input must raise NV");
}

#[test]
fn double_one_over_zero_is_infinity_raising_dz() {
    // f64: 1.0 / 0.0 should set DZ
    let one = f64::to_bits(1.0);
    let zero = f64::to_bits(0.0);
    let (result, flags) = fpu::execute(
        FpOp::Arith(FpArithOp::Div),
        [one, zero, 0],
        FpFormat::Double,
        RoundingMode::Rne,
    );

    let res_f64 = f64::from_bits(result);
    assert!(res_f64.is_infinite());
    assert!(flags.contains(FpFlags::DZ));
}

#[test]
fn double_sqrt_of_minus_one_raises_nv() {
    let neg_one = f64::to_bits(-1.0);
    let (_result, flags) =
        fpu::execute(FpOp::Sqrt, [neg_one, 0, 0], FpFormat::Double, RoundingMode::Rne);
    assert!(flags.contains(FpFlags::NV));
}

#[test]
fn nv_or_dz_contains_both_but_not_of() {
    let combined = FpFlags::NV | FpFlags::DZ;
    assert!(combined.contains(FpFlags::NV));
    assert!(combined.contains(FpFlags::DZ));
    assert!(!combined.contains(FpFlags::OF));
}

#[test]
fn the_flags_match_the_fflags_bit_layout() {
    assert_eq!(FpFlags::NV.bits(), 0b10000);
    assert_eq!(FpFlags::DZ.bits(), 0b01000);
    assert_eq!(FpFlags::OF.bits(), 0b00100);
    assert_eq!(FpFlags::UF.bits(), 0b00010);
    assert_eq!(FpFlags::NX.bits(), 0b00001);
    assert_eq!(FpFlags::NONE.bits(), 0);
}

const F16_ZERO: u16 = 0x0000;
const F16_INF: u16 = 0x7c00;
const F16_NEG_INF: u16 = 0xfc00;
const F16_ONE: u16 = 0x3c00;
const F16_QNAN: u16 = 0x7e00;

fn execute_f16(op: FpOp, a: u16, b: u16, c: u16) -> FpFlags {
    use crate::exec::compute::fpu::half::box_f16;
    use crate::isa::fp::{FpFormat, RoundingMode};
    let (_, flags) =
        fpu::execute(op, [box_f16(a), box_f16(b), box_f16(c)], FpFormat::Half, RoundingMode::Rne);
    flags
}

#[test]
fn a_half_precision_sum_of_opposite_infinities_is_invalid() {
    assert!(
        execute_f16(FpOp::Arith(FpArithOp::Add), F16_NEG_INF, F16_INF, 0).contains(FpFlags::NV)
    );
    assert!(execute_f16(FpOp::Arith(FpArithOp::Sub), F16_INF, F16_INF, 0).contains(FpFlags::NV));
    assert!(!execute_f16(FpOp::Arith(FpArithOp::Add), F16_INF, F16_INF, 0).contains(FpFlags::NV));
}

#[test]
fn a_half_precision_zero_times_infinity_is_invalid() {
    assert!(execute_f16(FpOp::Arith(FpArithOp::Mul), F16_ZERO, F16_INF, 0).contains(FpFlags::NV));
    assert!(!execute_f16(FpOp::Arith(FpArithOp::Mul), F16_ONE, F16_INF, 0).contains(FpFlags::NV));
}

#[test]
fn a_half_precision_fused_zero_times_infinity_is_invalid_even_with_a_quiet_nan_addend() {
    assert!(
        execute_f16(FpOp::Fma(FmaSigns::Macc), F16_ZERO, F16_INF, F16_QNAN).contains(FpFlags::NV)
    );
    assert!(
        execute_f16(FpOp::Fma(FmaSigns::Macc), F16_ONE, F16_INF, F16_NEG_INF).contains(FpFlags::NV)
    );
    assert!(
        !execute_f16(FpOp::Fma(FmaSigns::MSac), F16_ONE, F16_INF, F16_NEG_INF)
            .contains(FpFlags::NV)
    );
}

#[test]
fn a_half_precision_fma_whose_f64_step_rounds_away_the_product_is_inexact() {
    // -2^-24 × 2^-24 - 32 is -32 - 2^-48, half an f64 ulp below -32: the f64
    // fused step rounds it to -32 exactly, which converts to f16 exactly.
    let (smallest_negative, smallest, minus_32) = (0x8001, 0x0001, 0xd000);
    let flags = execute_f16(FpOp::Fma(FmaSigns::Macc), smallest_negative, smallest, minus_32);
    assert_eq!(flags, FpFlags::NX);
}

#[test]
fn a_half_result_below_the_smallest_normal_at_full_precision_underflows() {
    use crate::exec::compute::fpu::half::f64_to_f16;
    use crate::isa::fp::RoundingMode;
    // -(2^-14 - 2^-25 - 2^-30) rounds down to -(2^-14 - 2^-25) at f16's full
    // precision, which is tiny, though at subnormal precision it rounds to
    // -2^-14, the smallest normal.
    let just_below_smallest_normal = -(2f64.powi(-14) - 2f64.powi(-25) - 2f64.powi(-30));
    let (bits, flags) = f64_to_f16(just_below_smallest_normal, RoundingMode::Rdn);
    assert_eq!((bits, flags), (0x8400, FpFlags::NX | FpFlags::UF));
}
