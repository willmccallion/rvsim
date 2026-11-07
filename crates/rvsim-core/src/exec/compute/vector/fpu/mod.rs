//! Vector Floating-Point Unit.
//!
//! Implements all RISC-V Vector Extension (RVV 1.0) floating-point operations.
//! The main entry point [`vec_fp_execute`] dispatches to per-element loops that
//! handle masking, prestart/tail policy, NaN boxing, and FP exception flag
//! accumulation.
//!
//! Operations are grouped into categories:
//! - Arithmetic: add, sub, mul, div, sqrt, rsub, rdiv
//! - Min/max: IEEE 754-2008 minNum/maxNum
//! - Sign injection: sgnj, sgnjn, sgnjx
//! - Fused multiply-add: fmacc, fnmacc, fmsac, fnmsac, fmadd, fnmadd, fmsub, fnmsub
//! - Comparison (write mask): feq, fne, flt, fle, fgt, fge
//! - Classification: vfclass
//! - Conversions: int<->float, widening, narrowing
//! - Merge/move: vfmerge, vfmv.s.f, vfmv.f.s
//! - Slide: vfslide1up, vfslide1down

#![allow(clippy::float_cmp)]
mod arith;
mod compare;
mod convert;
mod estimate;
mod fma;
mod moves;
mod widen;

use crate::exec::compute::fpu::half::F16_NAN_BOX_MASK;
use crate::exec::compute::fpu::host::{restore_host_round_mode, set_host_round_mode};
use crate::exec::compute::fpu::nan_handling::F32_NAN_BOX_MASK;
use crate::exec::compute::fpu::{IntTarget, fp_to_int};
use crate::exec::compute::vector::context::{VecExecCtx, VecExecResult, VecOperand};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::{FpFlags, RoundingMode};
use crate::isa::op::{IntRounding, Signedness, VecClass, VecFpOp, VectorOp};
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax};
use arith::{StandardOp, exec_fp_standard};
use compare::exec_fp_comparison;
use fma::exec_fp_fma;
use moves::{SlideDirection, exec_fp_merge, exec_fp_slide1};
use widen::{exec_fp_narrowing, exec_fp_widen_convert, exec_fp_widening, exec_fp_widening_fma};

/// Returns `true` if `op` is a vector floating-point operation handled by this module.
pub const fn is_vec_fp(op: VectorOp) -> bool {
    matches!(op.class(), VecClass::Fp(_))
}

/// Execute a vector floating-point operation.
///
/// This is the main entry point for all vector FP operations. It dispatches to
/// specialised loops based on the operation category.
///
/// # Arguments
///
/// * `op`       - The vector floating-point operation to perform.
/// * `vpr`      - Mutable reference to the vector register file.
/// * `vd_idx`   - Destination vector register index.
/// * `vs2_idx`  - Second source vector register index.
/// * `operand1` - First operand (vector, scalar, or immediate).
/// * `ctx`      - Execution context (SEW, vl, masking policies, etc.).
pub fn vec_fp_execute(
    op: VecFpOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Set host FPU rounding mode from fcsr.frm for this instruction.
    let saved_rm = set_host_round_mode(ctx.frm);
    let result = vec_fp_dispatch(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    restore_host_round_mode(saved_rm);
    result
}

fn vec_fp_dispatch(
    op: VecFpOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match op {
        VecFpOp::Binary(op) => {
            exec_fp_standard(StandardOp::Binary(op), vpr, vd_idx, vs2_idx, operand1, ctx)
        }
        VecFpOp::Unary(op) => {
            exec_fp_standard(StandardOp::Unary(op), vpr, vd_idx, vs2_idx, operand1, ctx)
        }
        VecFpOp::Fma(signs, addend) => {
            exec_fp_fma((signs, addend), vpr, vd_idx, vs2_idx, operand1, ctx)
        }
        VecFpOp::Compare(op) => exec_fp_comparison(op, vpr, vd_idx, vs2_idx, operand1, ctx),
        VecFpOp::Widen(op) => exec_fp_widening(op, vpr, vd_idx, vs2_idx, operand1, ctx),
        VecFpOp::WidenFma(signs) => {
            exec_fp_widening_fma(signs, vpr, vd_idx, vs2_idx, operand1, ctx)
        }
        VecFpOp::WidenConvert(op) => exec_fp_widen_convert(op, vpr, vd_idx, vs2_idx, ctx),
        VecFpOp::NarrowConvert(op) => exec_fp_narrowing(op, vpr, vd_idx, vs2_idx, ctx),
        VecFpOp::Merge => exec_fp_merge(vpr, vd_idx, vs2_idx, operand1, ctx),
        VecFpOp::Slide1Up => {
            exec_fp_slide1(SlideDirection::Up, vpr, vd_idx, vs2_idx, operand1, ctx)
        }
        VecFpOp::Slide1Down => {
            exec_fp_slide1(SlideDirection::Down, vpr, vd_idx, vs2_idx, operand1, ctx)
        }
        VecFpOp::MoveToScalar => move_to_scalar(vpr, vs2_idx, ctx),
        VecFpOp::MoveFromScalar => move_from_scalar(vpr, vd_idx, operand1, ctx),
    }
}

/// `vfmv.f.s`: `vs2[0]` as the scalar result, NaN-boxed into the 64-bit
/// FP register when narrower (RISC-V spec §13.2).
fn move_to_scalar(vpr: &impl VectorRegFile, vs2_idx: VRegIdx, ctx: &VecExecCtx) -> VecExecResult {
    let raw = vpr.read_element(vs2_idx, ElemIdx::new(0), ctx.sew);
    let val = match ctx.sew {
        Sew::E32 => raw | F32_NAN_BOX_MASK,
        Sew::E16 => raw | F16_NAN_BOX_MASK,
        Sew::E8 | Sew::E64 => raw,
    };
    VecExecResult { scalar_result: Some(val), ..VecExecResult::default() }
}

/// `vfmv.s.f`: the scalar into `vd[0]`, the tail as its policy says.
fn move_from_scalar(
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let scalar = match operand1 {
        VecOperand::Scalar(s) => s,
        _ => 0,
    };
    if ctx.vl > 0 {
        vpr.write_element(vd_idx, ElemIdx::new(0), ctx.sew, scalar & ctx.sew.mask());
    }
    if ctx.vta.is_agnostic() {
        let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
        let start = if ctx.vl > 0 { 1 } else { 0 };
        for i in start..vlmax {
            vpr.fill_agnostic_element(vd_idx, ElemIdx::new(i), ctx.sew);
        }
    }
    VecExecResult::default()
}

/// `value` as a `bits`-wide integer of the signedness and rounding a
/// conversion names, with the flags.
fn float_to_int(
    value: f64,
    (signedness, rounding): (Signedness, IntRounding),
    bits: u32,
    frm: RoundingMode,
) -> (u64, FpFlags) {
    let target = match signedness {
        Signedness::Unsigned => IntTarget::unsigned(bits),
        Signedness::Signed => IntTarget::signed(bits),
    };
    let rm = match rounding {
        IntRounding::Dynamic => frm,
        IntRounding::TowardZero => RoundingMode::Rtz,
    };
    fp_to_int(value, target, rm)
}

/// Convert a raw u64 element to f32 (unboxed from lower 32 bits).
#[inline]
const fn elem_to_f32(val: u64) -> f32 {
    f32::from_bits(val as u32)
}

/// Convert a raw u64 element to f64.
#[inline]
const fn elem_to_f64(val: u64) -> f64 {
    f64::from_bits(val)
}

/// Bit mask for the sign bit in a 32-bit IEEE 754 float.
const F32_SIGN_BIT: u32 = 0x8000_0000;

/// Bit mask for the sign bit in a 64-bit IEEE 754 float.
const F64_SIGN_BIT: u64 = 0x8000_0000_0000_0000;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::arch::regs::vpr::Vpr;
    use crate::isa::fp::FpFlags;
    use crate::isa::fp::RoundingMode;
    use crate::isa::rvv::Vlen;

    /// The floating-point operation `op` decodes to.
    fn fp(op: VectorOp) -> VecFpOp {
        match op.class() {
            VecClass::Fp(op) => op,
            other => panic!("{op:?} is not a floating-point op: {other:?}"),
        }
    }

    fn vpr128() -> Vpr {
        Vpr::new(Vlen::new_unchecked(128))
    }

    /// The flags `op` raises on one Zvfh element pair `vs2[0]`, `vs1[0]`
    /// with `vd[0]` as an FMA's addend.
    fn f16_flags(op: VectorOp, vs2: u16, vs1: u16, vd: u16) -> FpFlags {
        let mut vpr = vpr128();
        let ctx = VecExecCtx { zvfh: true, ..VecExecCtx::unmasked(Sew::E16, 1) };
        let (v1, v2, v3) = (VRegIdx::new(1), VRegIdx::new(2), VRegIdx::new(3));
        vpr.write_element(v2, ElemIdx::new(0), Sew::E16, u64::from(vs2));
        vpr.write_element(v1, ElemIdx::new(0), Sew::E16, u64::from(vs1));
        vpr.write_element(v3, ElemIdx::new(0), Sew::E16, u64::from(vd));

        vec_fp_execute(fp(op), &mut vpr, v3, v2, VecOperand::Vector(v1), &ctx).fp_flags
    }

    /// The flags a widening `op` raises with a signaling NaN in `vs2[0]` and
    /// ones in `vs1[0]` and the (double-width) `vd[0]`.
    fn widening_flags_with_signaling_nan(op: VectorOp, sew: Sew, snan: u64, one: u64) -> FpFlags {
        let mut vpr = vpr128();
        let ctx = VecExecCtx { zvfh: true, ..VecExecCtx::unmasked(sew, 1) };
        let (v1, v2, v4) = (VRegIdx::new(1), VRegIdx::new(2), VRegIdx::new(4));
        vpr.write_element(v2, ElemIdx::new(0), sew, snan);
        vpr.write_element(v1, ElemIdx::new(0), sew, one);
        vpr.write_element(v4, ElemIdx::new(0), Sew::E64, 1f64.to_bits());
        vec_fp_execute(fp(op), &mut vpr, v4, v2, VecOperand::Vector(v1), &ctx).fp_flags
    }

    #[test]
    fn a_widening_add_of_a_single_precision_signaling_nan_is_invalid() {
        let flags = widening_flags_with_signaling_nan(
            VectorOp::VFWAdd,
            Sew::E32,
            0x7fa0_0000,
            u64::from(1f32.to_bits()),
        );

        assert!(flags.contains(FpFlags::NV));
    }

    #[test]
    fn a_widening_fma_of_a_single_precision_signaling_nan_is_invalid() {
        let flags = widening_flags_with_signaling_nan(
            VectorOp::VFWMacc,
            Sew::E32,
            0x7fa0_0000,
            u64::from(1f32.to_bits()),
        );

        assert!(flags.contains(FpFlags::NV));
    }

    #[test]
    fn a_zvfh_widening_add_of_a_signaling_nan_is_invalid() {
        let flags = widening_flags_with_signaling_nan(VectorOp::VFWAdd, Sew::E16, 0x7d00, 0x3c00);

        assert!(flags.contains(FpFlags::NV));
    }

    #[test]
    fn a_zvfh_widening_fma_of_a_signaling_nan_is_invalid() {
        let flags = widening_flags_with_signaling_nan(VectorOp::VFWMacc, Sew::E16, 0x7d00, 0x3c00);

        assert!(flags.contains(FpFlags::NV));
    }

    /// The value and flags a conversion `op` gives `vs2[0]` (at `src_sew`)
    /// written at `dst_sew`, at element width `sew` under `frm`.
    fn convert_one(
        op: VectorOp,
        sew: Sew,
        (src_sew, src): (Sew, u64),
        dst_sew: Sew,
        frm: RoundingMode,
    ) -> (u64, FpFlags) {
        convert_one_on(true, op, sew, (src_sew, src), dst_sew, frm)
    }

    /// [`convert_one`] on a unit with or without Zvfh.
    fn convert_one_on(
        zvfh: bool,
        op: VectorOp,
        sew: Sew,
        (src_sew, src): (Sew, u64),
        dst_sew: Sew,
        frm: RoundingMode,
    ) -> (u64, FpFlags) {
        let mut vpr = vpr128();
        let ctx = VecExecCtx { frm, zvfh, ..VecExecCtx::unmasked(sew, 1) };
        let (v2, v4) = (VRegIdx::new(2), VRegIdx::new(4));
        vpr.write_element(v2, ElemIdx::new(0), src_sew, src);
        let flags = vec_fp_execute(fp(op), &mut vpr, v4, v2, VecOperand::Vector(v2), &ctx).fp_flags;
        (vpr.read_element(v4, ElemIdx::new(0), dst_sew), flags)
    }

    #[test]
    fn a_widening_float_conversion_ignores_the_register_its_vs1_field_names() {
        // vfwcvt.f.f.v holds the VFUNARY0 selector 0b01100 in its vs1 field,
        // which decode passes on as the operand v12.
        let mut vpr = vpr128();
        let ctx = VecExecCtx::unmasked(Sew::E32, 1);
        let (v2, v4, v12) = (VRegIdx::new(2), VRegIdx::new(4), VRegIdx::new(12));
        vpr.write_element(v2, ElemIdx::new(0), Sew::E32, u64::from(1f32.to_bits()));
        vpr.write_element(v12, ElemIdx::new(0), Sew::E32, F32_SIGNALING_NAN);

        let flags =
            vec_fp_execute(fp(VectorOp::VFWCvtFF), &mut vpr, v4, v2, VecOperand::Vector(v12), &ctx)
                .fp_flags;

        assert_eq!(vpr.read_element(v4, ElemIdx::new(0), Sew::E64), 1f64.to_bits());
        assert_eq!(flags, FpFlags::NONE);
    }

    const F32_SIGNALING_NAN: u64 = 0x7f80_0001;
    const F16_THREE_AND_A_HALF: u64 = 0x4300;
    const F16_MINUS_TWO: u64 = 0xc000;
    const F16_MINUS_THREE: u64 = 0xc200;
    const F16_THREE_HUNDRED: u64 = 0x5cb0;

    #[test]
    fn an_8_bit_integer_widens_to_its_half_precision_value() {
        let (value, flags) =
            convert_one(VectorOp::VFWCvtFX, Sew::E8, (Sew::E8, 0xfd), Sew::E16, RoundingMode::Rne);

        assert_eq!((value, flags.bits()), (F16_MINUS_THREE, FpFlags::NONE.bits()));
    }

    #[test]
    fn half_precision_narrowing_to_signed_8_bits_rounds_per_frm_and_is_inexact() {
        let (value, flags) = convert_one(
            VectorOp::VFNCvtXF,
            Sew::E8,
            (Sew::E16, F16_THREE_AND_A_HALF),
            Sew::E8,
            RoundingMode::Rne,
        );

        assert_eq!((value, flags.bits()), (4, FpFlags::NX.bits()));
    }

    #[test]
    fn half_precision_narrowing_past_unsigned_8_bits_saturates_and_is_invalid() {
        let (value, flags) = convert_one(
            VectorOp::VFNCvtXuF,
            Sew::E8,
            (Sew::E16, F16_THREE_HUNDRED),
            Sew::E8,
            RoundingMode::Rne,
        );

        assert_eq!((value, flags.bits()), (255, FpFlags::NV.bits()));
    }

    #[test]
    fn a_16_bit_integer_widens_to_single_precision_without_zvfh() {
        let (value, _) = convert_one_on(
            false,
            VectorOp::VFWCvtFX,
            Sew::E16,
            (Sew::E16, 0xfffd),
            Sew::E32,
            RoundingMode::Rne,
        );

        assert_eq!(value, u64::from((-3f32).to_bits()));
    }

    #[test]
    fn single_precision_narrows_to_a_16_bit_integer_without_zvfh() {
        let (value, _) = convert_one_on(
            false,
            VectorOp::VFNCvtXF,
            Sew::E16,
            (Sew::E32, u64::from((-3f32).to_bits())),
            Sew::E16,
            RoundingMode::Rne,
        );

        assert_eq!(value, 0xfffd);
    }

    #[test]
    fn a_zvfh_widening_conversion_to_signed_rounds_per_frm_and_is_inexact() {
        let (value, flags) = convert_one(
            VectorOp::VFWCvtXF,
            Sew::E16,
            (Sew::E16, F16_THREE_AND_A_HALF),
            Sew::E32,
            RoundingMode::Rne,
        );

        assert_eq!((value, flags.bits()), (4, FpFlags::NX.bits()));
    }

    #[test]
    fn a_zvfh_widening_conversion_of_a_negative_value_to_unsigned_is_invalid() {
        let (value, flags) = convert_one(
            VectorOp::VFWCvtXuF,
            Sew::E16,
            (Sew::E16, F16_MINUS_TWO),
            Sew::E32,
            RoundingMode::Rne,
        );

        assert_eq!((value, flags.bits()), (0, FpFlags::NV.bits()));
    }

    #[test]
    fn a_narrowing_conversion_to_signed_rounds_per_frm_and_is_inexact() {
        let (value, flags) = convert_one(
            VectorOp::VFNCvtXF,
            Sew::E16,
            (Sew::E32, u64::from(3.5f32.to_bits())),
            Sew::E16,
            RoundingMode::Rup,
        );

        assert_eq!((value, flags.bits()), (4, FpFlags::NX.bits()));
    }

    #[test]
    fn a_narrowing_conversion_out_of_range_saturates_and_is_invalid() {
        let (value, flags) = convert_one(
            VectorOp::VFNCvtXF,
            Sew::E16,
            (Sew::E32, u64::from(70_000f32.to_bits())),
            Sew::E16,
            RoundingMode::Rne,
        );

        assert_eq!((value, flags.bits()), (0x7fff, FpFlags::NV.bits()));
    }

    #[test]
    fn a_narrowing_round_toward_zero_conversion_ignores_frm_and_is_inexact() {
        let (value, flags) = convert_one(
            VectorOp::VFNCvtRtzXuF,
            Sew::E16,
            (Sew::E32, u64::from(3.75f32.to_bits())),
            Sew::E16,
            RoundingMode::Rup,
        );

        assert_eq!((value, flags.bits()), (3, FpFlags::NX.bits()));
    }

    #[test]
    fn a_zvfh_widening_add_whose_sum_needs_more_than_single_precision_is_inexact() {
        let mut vpr = vpr128();
        let ctx = VecExecCtx { zvfh: true, ..VecExecCtx::unmasked(Sew::E16, 1) };
        let (v1, v2, v4) = (VRegIdx::new(1), VRegIdx::new(2), VRegIdx::new(4));
        let f16_two_to_15 = 0x7800;
        let f16_two_to_minus_24 = 0x0001;
        vpr.write_element(v2, ElemIdx::new(0), Sew::E16, f16_two_to_15);
        vpr.write_element(v1, ElemIdx::new(0), Sew::E16, f16_two_to_minus_24);

        let result =
            vec_fp_execute(fp(VectorOp::VFWAdd), &mut vpr, v4, v2, VecOperand::Vector(v1), &ctx);

        assert!(result.fp_flags.contains(FpFlags::NX));
        assert_eq!(vpr.read_element(v4, ElemIdx::new(0), Sew::E32), u64::from(32768f32.to_bits()));
    }

    #[test]
    fn a_zvfh_sum_of_opposite_infinities_is_invalid() {
        assert!(f16_flags(VectorOp::VFAdd, 0xfc00, 0x7c00, 0).contains(FpFlags::NV));
        assert!(f16_flags(VectorOp::VFSub, 0x7c00, 0x7c00, 0).contains(FpFlags::NV));
    }

    #[test]
    fn a_zvfh_zero_times_infinity_is_invalid() {
        assert!(f16_flags(VectorOp::VFMul, 0x0000, 0x7c00, 0).contains(FpFlags::NV));
    }

    #[test]
    fn a_zvfh_fused_zero_times_infinity_is_invalid_even_with_a_quiet_nan_addend() {
        assert!(f16_flags(VectorOp::VFMacc, 0x0000, 0x7c00, 0x7e00).contains(FpFlags::NV));
    }

    #[test]
    fn test_vfadd_f32() {
        let mut vpr = vpr128();
        let ctx = VecExecCtx::unmasked(Sew::E32, 4);
        let v1 = VRegIdx::new(1);
        let v2 = VRegIdx::new(2);
        let v3 = VRegIdx::new(3);

        // Write 2.0f32 to v2 elements and 3.0f32 to v1 elements
        for i in 0..4 {
            vpr.write_element(v2, ElemIdx::new(i), Sew::E32, 2.0f32.to_bits() as u64);
            vpr.write_element(v1, ElemIdx::new(i), Sew::E32, 3.0f32.to_bits() as u64);
        }

        let result =
            vec_fp_execute(fp(VectorOp::VFAdd), &mut vpr, v3, v2, VecOperand::Vector(v1), &ctx);

        assert!(!result.vxsat);
        for i in 0..4 {
            let val = vpr.read_element(v3, ElemIdx::new(i), Sew::E32);
            let f = f32::from_bits(val as u32);
            assert_eq!(f, 5.0);
        }
    }

    #[test]
    fn test_vfclass_f32() {
        let mut vpr = vpr128();
        let ctx = VecExecCtx::unmasked(Sew::E32, 4);
        let v1 = VRegIdx::new(1);
        let v2 = VRegIdx::new(2);

        // Write: +0.0, -0.0, +inf, qNaN
        vpr.write_element(v1, ElemIdx::new(0), Sew::E32, 0.0f32.to_bits() as u64);
        vpr.write_element(v1, ElemIdx::new(1), Sew::E32, (-0.0f32).to_bits() as u64);
        vpr.write_element(v1, ElemIdx::new(2), Sew::E32, f32::INFINITY.to_bits() as u64);
        vpr.write_element(v1, ElemIdx::new(3), Sew::E32, f32::NAN.to_bits() as u64);

        let _result =
            vec_fp_execute(fp(VectorOp::VFClass), &mut vpr, v2, v1, VecOperand::Scalar(0), &ctx);

        assert_eq!(vpr.read_element(v2, ElemIdx::new(0), Sew::E32), 1 << 4); // +zero
        assert_eq!(vpr.read_element(v2, ElemIdx::new(1), Sew::E32), 1 << 3); // -zero
        assert_eq!(vpr.read_element(v2, ElemIdx::new(2), Sew::E32), 1 << 7); // +inf
        assert_eq!(vpr.read_element(v2, ElemIdx::new(3), Sew::E32), 1 << 9); // qNaN
    }

    #[test]
    fn test_vfmv_sf() {
        let mut vpr = vpr128();
        let ctx = VecExecCtx::unmasked(Sew::E32, 4);
        let v1 = VRegIdx::new(1);

        let scalar = 42.0f32.to_bits() as u64;
        let result = vec_fp_execute(
            fp(VectorOp::VFMvSF),
            &mut vpr,
            v1,
            VRegIdx::new(0),
            VecOperand::Scalar(scalar),
            &ctx,
        );

        assert!(result.scalar_result.is_none());
        let val = vpr.read_element(v1, ElemIdx::new(0), Sew::E32);
        assert_eq!(f32::from_bits(val as u32), 42.0);
    }

    #[test]
    fn test_vfmv_fs() {
        let mut vpr = vpr128();
        let ctx = VecExecCtx::unmasked(Sew::E32, 4);
        let v1 = VRegIdx::new(1);

        vpr.write_element(v1, ElemIdx::new(0), Sew::E32, 99.5f32.to_bits() as u64);

        let result = vec_fp_execute(
            fp(VectorOp::VFMvFS),
            &mut vpr,
            VRegIdx::new(2),
            v1,
            VecOperand::Scalar(0),
            &ctx,
        );

        let scalar = result.scalar_result.unwrap();
        assert_eq!(f32::from_bits(scalar as u32), 99.5);
    }
}
