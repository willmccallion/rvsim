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

use crate::exec::compute::fpu::{restore_host_round_mode, set_host_round_mode};
use crate::exec::compute::vector::context::{VecExecCtx, VecExecResult, VecOperand};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::FpFlags;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax};
use arith::exec_fp_standard;
use compare::exec_fp_comparison;
use fma::exec_fp_fma;
use moves::{exec_fp_merge, exec_fp_slide1};
use widen::{exec_fp_narrowing, exec_fp_widening};

/// Returns `true` if `op` is a vector floating-point operation handled by this module.
pub const fn is_vec_fp(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VFAdd
            | VectorOp::VFSub
            | VectorOp::VFRSub
            | VectorOp::VFMul
            | VectorOp::VFDiv
            | VectorOp::VFRDiv
            | VectorOp::VFMin
            | VectorOp::VFMax
            | VectorOp::VFSgnj
            | VectorOp::VFSgnjn
            | VectorOp::VFSgnjx
            | VectorOp::VMFEq
            | VectorOp::VMFNe
            | VectorOp::VMFLt
            | VectorOp::VMFLe
            | VectorOp::VMFGt
            | VectorOp::VMFGe
            | VectorOp::VFMacc
            | VectorOp::VFNMacc
            | VectorOp::VFMSac
            | VectorOp::VFNMSac
            | VectorOp::VFMAdd
            | VectorOp::VFNMAdd
            | VectorOp::VFMSub
            | VectorOp::VFNMSub
            | VectorOp::VFSqrt
            | VectorOp::VFRsqrt7
            | VectorOp::VFRec7
            | VectorOp::VFClass
            | VectorOp::VFCvtXuF
            | VectorOp::VFCvtXF
            | VectorOp::VFCvtFXu
            | VectorOp::VFCvtFX
            | VectorOp::VFCvtRtzXuF
            | VectorOp::VFCvtRtzXF
            | VectorOp::VFWAdd
            | VectorOp::VFWSub
            | VectorOp::VFWMul
            | VectorOp::VFWAddW
            | VectorOp::VFWSubW
            | VectorOp::VFWMacc
            | VectorOp::VFWNMacc
            | VectorOp::VFWMSac
            | VectorOp::VFWNMSac
            | VectorOp::VFWCvtXuF
            | VectorOp::VFWCvtXF
            | VectorOp::VFWCvtFXu
            | VectorOp::VFWCvtFX
            | VectorOp::VFWCvtFF
            | VectorOp::VFWCvtRtzXuF
            | VectorOp::VFWCvtRtzXF
            | VectorOp::VFNCvtXuF
            | VectorOp::VFNCvtXF
            | VectorOp::VFNCvtFXu
            | VectorOp::VFNCvtFX
            | VectorOp::VFNCvtFF
            | VectorOp::VFNCvtRodFF
            | VectorOp::VFNCvtRtzXuF
            | VectorOp::VFNCvtRtzXF
            | VectorOp::VFMerge
            | VectorOp::VFMvSF
            | VectorOp::VFMvFS
            | VectorOp::VFSlide1Up
            | VectorOp::VFSlide1Down
    )
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
    op: VectorOp,
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

#[allow(clippy::too_many_lines)]
fn vec_fp_dispatch(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Scalar move: vfmv.f.s — read vs2[0] as scalar result
    // The result is written to a 64-bit FP register; NaN-box sub-64-bit values
    // per RISC-V spec §13.2 (upper bits all-1s for narrower FP widths).
    if op == VectorOp::VFMvFS {
        let raw = vpr.read_element(vs2_idx, ElemIdx::new(0), ctx.sew);
        let val = match ctx.sew {
            Sew::E32 => raw | 0xFFFF_FFFF_0000_0000, // NaN-box f32 in f64 register
            Sew::E16 => raw | 0xFFFF_FFFF_FFFF_0000, // NaN-box f16 in f64 register
            _ => raw,                                // E64: no boxing needed
        };
        return VecExecResult { vxsat: false, scalar_result: Some(val), fp_flags: FpFlags::NONE };
    }

    // Scalar move: vfmv.s.f — write scalar into vd[0]
    if op == VectorOp::VFMvSF {
        let scalar = match operand1 {
            VecOperand::Scalar(s) => s,
            _ => 0,
        };
        if ctx.vl > 0 {
            vpr.write_element(vd_idx, ElemIdx::new(0), ctx.sew, scalar & ctx.sew.mask());
        }
        // Tail elements follow the tail-agnostic rule.
        if ctx.vta.is_agnostic() {
            let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
            let start = if ctx.vl > 0 { 1 } else { 0 };
            for i in start..vlmax {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
        }
        return VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE };
    }

    // Merge
    if op == VectorOp::VFMerge {
        return exec_fp_merge(vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // Slide operations
    if matches!(op, VectorOp::VFSlide1Up | VectorOp::VFSlide1Down) {
        return exec_fp_slide1(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // Comparisons (write mask bits)
    if is_fp_comparison(op) {
        return exec_fp_comparison(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // Widening operations
    if is_fp_widening(op) {
        return exec_fp_widening(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // Narrowing operations
    if is_fp_narrowing(op) {
        return exec_fp_narrowing(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // FMA operations (need vd as accumulator)
    if is_fp_fma(op) {
        return exec_fp_fma(op, vpr, vd_idx, vs2_idx, operand1, ctx);
    }

    // Standard element-wise FP operations
    exec_fp_standard(op, vpr, vd_idx, vs2_idx, operand1, ctx)
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

/// Returns `true` for FP comparison ops that write mask bits.
const fn is_fp_comparison(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VMFEq
            | VectorOp::VMFNe
            | VectorOp::VMFLt
            | VectorOp::VMFLe
            | VectorOp::VMFGt
            | VectorOp::VMFGe
    )
}

/// Returns `true` for FP widening operations.
const fn is_fp_widening(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VFWAdd
            | VectorOp::VFWSub
            | VectorOp::VFWMul
            | VectorOp::VFWAddW
            | VectorOp::VFWSubW
            | VectorOp::VFWMacc
            | VectorOp::VFWNMacc
            | VectorOp::VFWMSac
            | VectorOp::VFWNMSac
            | VectorOp::VFWCvtXuF
            | VectorOp::VFWCvtXF
            | VectorOp::VFWCvtFXu
            | VectorOp::VFWCvtFX
            | VectorOp::VFWCvtFF
            | VectorOp::VFWCvtRtzXuF
            | VectorOp::VFWCvtRtzXF
    )
}

/// Returns `true` for FP narrowing operations.
const fn is_fp_narrowing(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VFNCvtXuF
            | VectorOp::VFNCvtXF
            | VectorOp::VFNCvtFXu
            | VectorOp::VFNCvtFX
            | VectorOp::VFNCvtFF
            | VectorOp::VFNCvtRodFF
            | VectorOp::VFNCvtRtzXuF
            | VectorOp::VFNCvtRtzXF
    )
}

/// Returns `true` for FMA operations (need vd as accumulator).
const fn is_fp_fma(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VFMacc
            | VectorOp::VFNMacc
            | VectorOp::VFMSac
            | VectorOp::VFNMSac
            | VectorOp::VFMAdd
            | VectorOp::VFNMAdd
            | VectorOp::VFMSub
            | VectorOp::VFNMSub
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::arch::regs::vpr::Vpr;
    use crate::isa::fp::RoundingMode;
    use crate::isa::rvv::{MaskPolicy, TailPolicy, Vlen, Vlmul, Vxrm};

    fn make_ctx(sew: Sew, vl: usize) -> VecExecCtx {
        VecExecCtx {
            sew,
            vl,
            vstart: 0,
            vma: MaskPolicy::Undisturbed,
            vta: TailPolicy::Undisturbed,
            vlmul: Vlmul::M1,
            vm: true,
            vxrm: Vxrm::RoundToNearestUp,
            frm: RoundingMode::Rne,
            zvfh: false,
        }
    }

    fn vpr128() -> Vpr {
        Vpr::new(Vlen::new_unchecked(128))
    }

    #[test]
    fn test_vfadd_f32() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let v1 = VRegIdx::new(1);
        let v2 = VRegIdx::new(2);
        let v3 = VRegIdx::new(3);

        // Write 2.0f32 to v2 elements and 3.0f32 to v1 elements
        for i in 0..4 {
            vpr.write_element(v2, ElemIdx::new(i), Sew::E32, 2.0f32.to_bits() as u64);
            vpr.write_element(v1, ElemIdx::new(i), Sew::E32, 3.0f32.to_bits() as u64);
        }

        let result =
            vec_fp_execute(VectorOp::VFAdd, &mut vpr, v3, v2, VecOperand::Vector(v1), &ctx);

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
        let ctx = make_ctx(Sew::E32, 4);
        let v1 = VRegIdx::new(1);
        let v2 = VRegIdx::new(2);

        // Write: +0.0, -0.0, +inf, qNaN
        vpr.write_element(v1, ElemIdx::new(0), Sew::E32, 0.0f32.to_bits() as u64);
        vpr.write_element(v1, ElemIdx::new(1), Sew::E32, (-0.0f32).to_bits() as u64);
        vpr.write_element(v1, ElemIdx::new(2), Sew::E32, f32::INFINITY.to_bits() as u64);
        vpr.write_element(v1, ElemIdx::new(3), Sew::E32, f32::NAN.to_bits() as u64);

        let _result =
            vec_fp_execute(VectorOp::VFClass, &mut vpr, v2, v1, VecOperand::Scalar(0), &ctx);

        assert_eq!(vpr.read_element(v2, ElemIdx::new(0), Sew::E32), 1 << 4); // +zero
        assert_eq!(vpr.read_element(v2, ElemIdx::new(1), Sew::E32), 1 << 3); // -zero
        assert_eq!(vpr.read_element(v2, ElemIdx::new(2), Sew::E32), 1 << 7); // +inf
        assert_eq!(vpr.read_element(v2, ElemIdx::new(3), Sew::E32), 1 << 9); // qNaN
    }

    #[test]
    fn test_vfmv_sf() {
        let mut vpr = vpr128();
        let ctx = make_ctx(Sew::E32, 4);
        let v1 = VRegIdx::new(1);

        let scalar = 42.0f32.to_bits() as u64;
        let result = vec_fp_execute(
            VectorOp::VFMvSF,
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
        let ctx = make_ctx(Sew::E32, 4);
        let v1 = VRegIdx::new(1);

        vpr.write_element(v1, ElemIdx::new(0), Sew::E32, 99.5f32.to_bits() as u64);

        let result = vec_fp_execute(
            VectorOp::VFMvFS,
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
