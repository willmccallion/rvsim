//! RISC-V Rounding Mode tests.
//!
//! These tests verify that `fpu::execute()` correctly applies
//! each of the five RISC-V rounding modes.

use crate::exec::compute::fpu;
use crate::exec::compute::fpu::float::Float;
use crate::isa::fp::{FpFormat, RoundingMode};
use crate::isa::op::{FpArithOp, FpCompareOp, FpOp, SignInjectOp};

/// `value` NaN-boxed in a 64-bit FP register.
fn to_register(value: f32) -> u64 {
    Float::to_register(value)
}

/// Helper: box two f32 values, execute with rounding mode, unbox result.
fn fadd_f32_rm(a: f32, b: f32, rm: RoundingMode) -> f32 {
    let ba = to_register(a);
    let bb = to_register(b);
    let result = fpu::execute(FpOp::Arith(FpArithOp::Add), [ba, bb, 0], FpFormat::Single, rm).0;
    f32::from_bits(result as u32)
}

#[test]
fn rne_adds_exactly_representable_values_exactly() {
    // RNE is the IEEE default; matches Rust's native f32 add for exact results.
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rne);
    assert_eq!(result, 3.0);

    // Large values that are exactly representable
    let result = fadd_f32_rm(1.0e30, 1.0e30, RoundingMode::Rne);
    assert_eq!(result, 2.0e30);
}

/// Half an ulp of 1.0: `1.0 + HALF_ULP` is a tie between 1.0 and `ABOVE_ONE`.
const HALF_ULP: f32 = f32::from_bits(0x3380_0000);
/// Three quarters of an ulp of 1.0, nearer `ABOVE_ONE` than 1.0.
const THREE_QUARTER_ULP: f32 = f32::from_bits(0x33C0_0000);
/// The next f32 above 1.0.
const ABOVE_ONE: f32 = f32::from_bits(0x3F80_0001);

#[test]
fn each_rounding_mode_rounds_an_inexact_sum_its_own_way() {
    use RoundingMode::{Rdn, Rmm, Rne, Rtz, Rup};
    let cases = [
        (1.0, HALF_ULP, [(Rne, 1.0), (Rtz, 1.0), (Rdn, 1.0), (Rup, ABOVE_ONE), (Rmm, ABOVE_ONE)]),
        (
            -1.0,
            -HALF_ULP,
            [(Rne, -1.0), (Rtz, -1.0), (Rdn, -ABOVE_ONE), (Rup, -1.0), (Rmm, -ABOVE_ONE)],
        ),
        (
            1.0,
            THREE_QUARTER_ULP,
            [(Rne, ABOVE_ONE), (Rtz, 1.0), (Rdn, 1.0), (Rup, ABOVE_ONE), (Rmm, ABOVE_ONE)],
        ),
    ];

    for (a, b, rounded) in cases {
        for (rm, expected) in rounded {
            assert_eq!(fadd_f32_rm(a, b, rm), expected, "{a} + {b:e} under {rm:?}");
        }
    }
}

#[test]
fn rounding_mode_from_bits_valid() {
    assert_eq!(RoundingMode::from_bits(0b000), Some(RoundingMode::Rne));
    assert_eq!(RoundingMode::from_bits(0b001), Some(RoundingMode::Rtz));
    assert_eq!(RoundingMode::from_bits(0b010), Some(RoundingMode::Rdn));
    assert_eq!(RoundingMode::from_bits(0b011), Some(RoundingMode::Rup));
    assert_eq!(RoundingMode::from_bits(0b100), Some(RoundingMode::Rmm));
}

#[test]
fn rounding_mode_from_bits_reserved() {
    assert_eq!(RoundingMode::from_bits(0b101), None);
    assert_eq!(RoundingMode::from_bits(0b110), None);
}

#[test]
fn rounding_mode_from_bits_dynamic() {
    // 0b111 = dynamic, should return None (caller resolves from fcsr.frm)
    assert_eq!(RoundingMode::from_bits(0b111), None);
}

#[test]
fn rounding_mode_irrelevant_for_comparisons() {
    let a = to_register(1.0);
    let b = to_register(2.0);
    // FEq should return the same result regardless of rounding mode
    for rm in [
        RoundingMode::Rne,
        RoundingMode::Rtz,
        RoundingMode::Rdn,
        RoundingMode::Rup,
        RoundingMode::Rmm,
    ] {
        let result =
            fpu::execute(FpOp::Compare(FpCompareOp::Eq), [a, b, 0], FpFormat::Single, rm).0;
        assert_eq!(result, 0, "FEq(1.0, 2.0) should be 0 for all rounding modes");
    }
}

#[test]
fn rounding_mode_irrelevant_for_sign_injection() {
    #[allow(clippy::approx_constant)]
    let pos = to_register(3.14);
    let neg = to_register(-1.0);
    for rm in [
        RoundingMode::Rne,
        RoundingMode::Rtz,
        RoundingMode::Rdn,
        RoundingMode::Rup,
        RoundingMode::Rmm,
    ] {
        let result =
            fpu::execute(FpOp::SignInject(SignInjectOp::Copy), [pos, neg, 0], FpFormat::Single, rm)
                .0;
        let res_f32 = f32::from_bits(result as u32);
        assert!(res_f32.is_sign_negative(), "FSgnJ(+, -) should produce negative");
        #[allow(clippy::approx_constant)]
        {
            assert!((res_f32.abs() - 3.14f32).abs() < 1e-6);
        }
    }
}
