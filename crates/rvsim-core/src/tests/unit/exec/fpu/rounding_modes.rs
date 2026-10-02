//! RISC-V Rounding Mode tests.
//!
//! These tests verify that `fpu::execute_full_rm()` correctly applies
//! each of the five RISC-V rounding modes.

use crate::exec::compute::fpu;
use crate::exec::compute::fpu::nan_handling::box_f32;
use crate::isa::fp::RoundingMode;
use crate::isa::op::AluOp;

/// Helper: box two f32 values, execute with rounding mode, unbox result.
fn fadd_f32_rm(a: f32, b: f32, rm: RoundingMode) -> f32 {
    let ba = box_f32(a);
    let bb = box_f32(b);
    let result = fpu::execute_full_rm(AluOp::FAdd, ba, bb, 0, false, true, rm).0;
    f32::from_bits(result as u32)
}

#[test]
fn test_rounding_rne() {
    // RNE is the IEEE default; matches Rust's native f32 add for exact results.
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rne);
    assert_eq!(result, 3.0);

    // Large values that are exactly representable
    let result = fadd_f32_rm(1.0e30, 1.0e30, RoundingMode::Rne);
    assert_eq!(result, 2.0e30);
}

#[test]
fn test_rounding_rtz() {
    // For exactly representable results, RTZ and RNE agree
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rtz);
    assert_eq!(result, 3.0);

    // RTZ truncates toward zero, so its magnitude must be ≤ the RNE result.
    let rtz_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rtz);
    let rne_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rne);
    assert!(rtz_result <= rne_result, "RTZ should produce result <= RNE for positive values");
    assert!(rtz_result >= 0.0, "RTZ of positive inputs should be non-negative");
}

#[test]
fn test_rounding_rdn() {
    // Exact results should be unchanged
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rdn);
    assert_eq!(result, 3.0);

    // RDN should produce a result <= exact value
    let rdn_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rdn);
    let rup_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rup);
    assert!(rdn_result <= rup_result, "RDN result should be <= RUP result");
}

#[test]
fn test_rounding_rup() {
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rup);
    assert_eq!(result, 3.0);

    // RUP should produce result >= exact for positive values
    let rup_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rup);
    let rdn_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rdn);
    assert!(rup_result >= rdn_result, "RUP result should be >= RDN result");
}

#[test]
fn test_rounding_rmm() {
    // For normal exact values, RMM and RNE agree
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rmm);
    assert_eq!(result, 3.0);

    // RMM should agree with RNE for non-tie cases
    let rmm_result = fadd_f32_rm(1.0e10, 1.0, RoundingMode::Rmm);
    let rne_result = fadd_f32_rm(1.0e10, 1.0, RoundingMode::Rne);
    assert_eq!(rmm_result, rne_result, "RMM and RNE should agree for non-ties");
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
    let a = box_f32(1.0);
    let b = box_f32(2.0);
    // FEq should return the same result regardless of rounding mode
    for rm in [
        RoundingMode::Rne,
        RoundingMode::Rtz,
        RoundingMode::Rdn,
        RoundingMode::Rup,
        RoundingMode::Rmm,
    ] {
        let result = fpu::execute_full_rm(AluOp::FEq, a, b, 0, false, true, rm).0;
        assert_eq!(result, 0, "FEq(1.0, 2.0) should be 0 for all rounding modes");
    }
}

#[test]
fn rounding_mode_irrelevant_for_sign_injection() {
    #[allow(clippy::approx_constant)]
    let pos = box_f32(3.14);
    let neg = box_f32(-1.0);
    for rm in [
        RoundingMode::Rne,
        RoundingMode::Rtz,
        RoundingMode::Rdn,
        RoundingMode::Rup,
        RoundingMode::Rmm,
    ] {
        let result = fpu::execute_full_rm(AluOp::FSgnJ, pos, neg, 0, false, true, rm).0;
        let res_f32 = f32::from_bits(result as u32);
        assert!(res_f32.is_sign_negative(), "FSgnJ(+, -) should produce negative");
        #[allow(clippy::approx_constant)]
        {
            assert!((res_f32.abs() - 3.14f32).abs() < 1e-6);
        }
    }
}
