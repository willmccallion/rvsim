//! RISC-V Rounding Mode tests.
//!
//! These tests verify that `Fpu::execute_with_rm()` correctly applies
//! each of the five RISC-V rounding modes.

use riscv_core::core::pipeline::signals::AluOp;
use riscv_core::core::units::fpu::Fpu;
use riscv_core::core::units::fpu::rounding_modes::RoundingMode;

/// Helper: box two f32 values, execute with rounding mode, unbox result.
fn fadd_f32_rm(a: f32, b: f32, rm: RoundingMode) -> f32 {
    let ba = Fpu::box_f32(a);
    let bb = Fpu::box_f32(b);
    let result = Fpu::execute_with_rm(AluOp::FAdd, ba, bb, 0, true, rm);
    f32::from_bits(result as u32)
}

// ══════════════════════════════════════════════════════════
// 1. RNE (Round to Nearest, ties to Even)
// ══════════════════════════════════════════════════════════

#[test]
fn test_rounding_rne() {
    // RNE is the default IEEE mode. Normal addition should produce
    // the same result as Rust's default.
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rne);
    assert_eq!(result, 3.0);

    // Large values that are exactly representable
    let result = fadd_f32_rm(1.0e30, 1.0e30, RoundingMode::Rne);
    assert_eq!(result, 2.0e30);
}

// ══════════════════════════════════════════════════════════
// 2. RTZ (Round towards Zero)
// ══════════════════════════════════════════════════════════

#[test]
fn test_rounding_rtz() {
    // For exactly representable results, RTZ and RNE agree
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rtz);
    assert_eq!(result, 3.0);

    // RTZ should truncate towards zero for positive values
    // The magnitude should be <= the RNE result
    let rtz_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rtz);
    let rne_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rne);
    assert!(
        rtz_result <= rne_result,
        "RTZ should produce result <= RNE for positive values"
    );
    assert!(
        rtz_result >= 0.0,
        "RTZ of positive inputs should be non-negative"
    );
}

// ══════════════════════════════════════════════════════════
// 3. RDN (Round Down, towards -infinity)
// ══════════════════════════════════════════════════════════

#[test]
fn test_rounding_rdn() {
    // Exact results should be unchanged
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rdn);
    assert_eq!(result, 3.0);

    // RDN should produce a result <= exact value
    let rdn_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rdn);
    let rup_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rup);
    assert!(
        rdn_result <= rup_result,
        "RDN result should be <= RUP result"
    );
}

// ══════════════════════════════════════════════════════════
// 4. RUP (Round Up, towards +infinity)
// ══════════════════════════════════════════════════════════

#[test]
fn test_rounding_rup() {
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rup);
    assert_eq!(result, 3.0);

    // RUP should produce result >= exact for positive values
    let rup_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rup);
    let rdn_result = fadd_f32_rm(1.0e-38, 1.0e-38, RoundingMode::Rdn);
    assert!(
        rup_result >= rdn_result,
        "RUP result should be >= RDN result"
    );
}

// ══════════════════════════════════════════════════════════
// 5. RMM (Round to Nearest, ties to Max Magnitude)
// ══════════════════════════════════════════════════════════

#[test]
fn test_rounding_rmm() {
    // For normal exact values, RMM and RNE agree
    let result = fadd_f32_rm(1.0, 2.0, RoundingMode::Rmm);
    assert_eq!(result, 3.0);

    // RMM should agree with RNE for non-tie cases
    let rmm_result = fadd_f32_rm(1.0e10, 1.0, RoundingMode::Rmm);
    let rne_result = fadd_f32_rm(1.0e10, 1.0, RoundingMode::Rne);
    assert_eq!(
        rmm_result, rne_result,
        "RMM and RNE should agree for non-ties"
    );
}

// ══════════════════════════════════════════════════════════
// 6. RoundingMode::from_bits decoding
// ══════════════════════════════════════════════════════════

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

// ══════════════════════════════════════════════════════════
// 7. Non-arithmetic ops ignore rounding mode
// ══════════════════════════════════════════════════════════

#[test]
fn rounding_mode_irrelevant_for_comparisons() {
    let a = Fpu::box_f32(1.0);
    let b = Fpu::box_f32(2.0);
    // FEq should return the same result regardless of rounding mode
    for rm in [
        RoundingMode::Rne,
        RoundingMode::Rtz,
        RoundingMode::Rdn,
        RoundingMode::Rup,
        RoundingMode::Rmm,
    ] {
        let result = Fpu::execute_with_rm(AluOp::FEq, a, b, 0, true, rm);
        assert_eq!(
            result, 0,
            "FEq(1.0, 2.0) should be 0 for all rounding modes"
        );
    }
}

#[test]
fn rounding_mode_irrelevant_for_sign_injection() {
    #[allow(clippy::approx_constant)]
    let pos = Fpu::box_f32(3.14);
    let neg = Fpu::box_f32(-1.0);
    for rm in [
        RoundingMode::Rne,
        RoundingMode::Rtz,
        RoundingMode::Rdn,
        RoundingMode::Rup,
        RoundingMode::Rmm,
    ] {
        let result = Fpu::execute_with_rm(AluOp::FSgnJ, pos, neg, 0, true, rm);
        let res_f32 = f32::from_bits(result as u32);
        assert!(
            res_f32.is_sign_negative(),
            "FSgnJ(+, -) should produce negative"
        );
        #[allow(clippy::approx_constant)]
        {
            assert!((res_f32.abs() - 3.14f32).abs() < 1e-6);
        }
    }
}
