//! Vector permutation operations.
//!
//! Implements all RISC-V Vector Extension (RVV 1.0) permutation operations:
//! scalar moves (`vmv.x.s`, `vmv.s.x`), slides (`vslideup`, `vslidedown`,
//! `vslide1up`, `vslide1down`), gathers (`vrgather`, `vrgatherei16`),
//! compress (`vcompress`), and whole-register moves (`vmv<n>r`).
//!
//! The main entry point [`vec_permute_execute`] dispatches to the appropriate
//! operation based on the [`VectorOp`] variant.

use crate::core::arch::vpr::Vpr;
use crate::core::pipeline::signals::VectorOp;
use crate::core::units::fpu::exception_flags::FpFlags;
use crate::core::units::vpu::alu::{VecExecCtx, VecExecResult, VecOperand};
use crate::core::units::vpu::types::{ElemIdx, MaskPolicy, Sew, TailPolicy, VRegIdx, Vlmax};

// ============================================================================
// Public API
// ============================================================================

/// Returns `true` if `op` is a permutation operation handled by this module.
pub const fn is_permute(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VMvXS
            | VectorOp::VMvSX
            | VectorOp::VSlideUp
            | VectorOp::VSlideDown
            | VectorOp::VSlide1Up
            | VectorOp::VSlide1Down
            | VectorOp::VRgather
            | VectorOp::VRgatherEi16
            | VectorOp::VCompress
            | VectorOp::VMv1r
            | VectorOp::VMv2r
            | VectorOp::VMv4r
            | VectorOp::VMv8r
    )
}

/// Execute a permutation operation.
///
/// For scalar-producing ops ([`VectorOp::VMvXS`]), the scalar value is
/// returned in [`VecExecResult::scalar_result`].  For vector-producing ops,
/// results are written directly to `vd` in the VPR.
pub fn vec_permute_execute(
    op: VectorOp,
    vpr: &mut Vpr,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match op {
        VectorOp::VMvXS => exec_vmv_xs(vpr, vs2, ctx),
        VectorOp::VMvSX => exec_vmv_sx(vpr, vd, operand1, ctx),
        VectorOp::VSlideUp => exec_slideup(vpr, vd, vs2, operand1, ctx),
        VectorOp::VSlideDown => exec_slidedown(vpr, vd, vs2, operand1, ctx),
        VectorOp::VSlide1Up => exec_slide1up(vpr, vd, vs2, operand1, ctx),
        VectorOp::VSlide1Down => exec_slide1down(vpr, vd, vs2, operand1, ctx),
        VectorOp::VRgather => exec_rgather(vpr, vd, vs2, operand1, ctx),
        VectorOp::VRgatherEi16 => exec_rgather_ei16(vpr, vd, vs2, operand1, ctx),
        VectorOp::VCompress => exec_compress(vpr, vd, vs2, ctx),
        VectorOp::VMv1r => exec_whole_reg_move(vpr, vd, vs2, 1),
        VectorOp::VMv2r => exec_whole_reg_move(vpr, vd, vs2, 2),
        VectorOp::VMv4r => exec_whole_reg_move(vpr, vd, vs2, 4),
        VectorOp::VMv8r => exec_whole_reg_move(vpr, vd, vs2, 8),
        _ => unreachable!("not a permutation op: {:?}", op),
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Read v0 mask bit for element `i`.
#[inline]
fn mask_active(vpr: &Vpr, i: usize) -> bool {
    vpr.read_mask_bit(VRegIdx::new(0), ElemIdx::new(i))
}

/// Write all-1s at the given SEW width to a destination element.
#[inline]
fn write_ones(vpr: &mut Vpr, vd: VRegIdx, i: usize, sew: Sew) {
    vpr.write_element(vd, ElemIdx::new(i), sew, sew.mask());
}

/// Extract the offset value from operand1 (scalar or immediate).
///
/// For `VecOperand::Scalar(v)`, the offset is `v`.
/// For `VecOperand::Immediate(v)`, the offset is the unsigned (zero-extended)
/// value.  Vector operands are not valid for slide offsets and will panic in
/// debug builds.
#[inline]
fn offset_from_operand(operand1: &VecOperand) -> usize {
    match operand1 {
        VecOperand::Scalar(v) => *v as usize,
        VecOperand::Immediate(v) => *v as u64 as usize,
        VecOperand::Vector(_) => unreachable!("slide offset must be scalar or immediate"),
    }
}

/// Extract a scalar value from operand1 at the given SEW width.
#[inline]
fn scalar_from_operand(operand1: &VecOperand, sew: Sew) -> u64 {
    match operand1 {
        VecOperand::Scalar(v) => *v & sew.mask(),
        VecOperand::Immediate(v) => (*v as u64) & sew.mask(),
        VecOperand::Vector(_) => unreachable!("expected scalar or immediate operand"),
    }
}

/// Build a default result with no flags set.
#[inline]
const fn no_flags_result(scalar: Option<u64>) -> VecExecResult {
    VecExecResult { vxsat: false, scalar_result: scalar, fp_flags: FpFlags::NONE }
}

// ============================================================================
// Scalar moves
// ============================================================================

/// `vmv.x.s` — move vs2[0] to a scalar GPR result.
///
/// Reads element 0 of `vs2` at the current SEW and returns it as the scalar
/// result.  Does not write any vector register.
fn exec_vmv_xs(vpr: &Vpr, vs2: VRegIdx, ctx: &VecExecCtx) -> VecExecResult {
    let val = vpr.read_element(vs2, ElemIdx::new(0), ctx.sew);
    no_flags_result(Some(val))
}

/// `vmv.s.x` — move a scalar GPR value into vd[0].
///
/// Writes the scalar to element 0 of `vd` at the current SEW.  Remaining
/// elements (indices 1..vlmax) follow the tail policy.
fn exec_vmv_sx(
    vpr: &mut Vpr,
    vd: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let scalar = scalar_from_operand(operand1, ctx.sew);

    // Write element 0 if vl > 0.
    if ctx.vl > 0 {
        vpr.write_element(vd, ElemIdx::new(0), ctx.sew, scalar);
    }

    // Tail policy for elements 1..vlmax.
    if matches!(ctx.vta, TailPolicy::Agnostic) {
        for i in 1..vlmax {
            write_ones(vpr, vd, i, ctx.sew);
        }
    }

    no_flags_result(None)
}

// ============================================================================
// Slides
// ============================================================================

/// `vslideup` — slide elements up by a given offset.
///
/// For each element i in [vstart, vl):
/// - If i < offset: element is unchanged (left undisturbed).
/// - If i >= offset: vd[i] = vs2[i - offset].
///
/// Elements in [vl, vlmax) follow the tail policy.  Masking applies normally.
fn exec_slideup(
    vpr: &mut Vpr,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let offset = offset_from_operand(operand1);

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            // Tail element.
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }
        // Active body element.
        if !ctx.vm && !mask_active(vpr, i) {
            // Masked-off: apply mask policy.
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }
        if i >= offset {
            let src_idx = i - offset;
            let val = vpr.read_element(vs2, ElemIdx::new(src_idx), ctx.sew);
            vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
        }
        // i < offset: leave unchanged.
    }

    no_flags_result(None)
}

/// `vslidedown` — slide elements down by a given offset.
///
/// For each element i in [vstart, vl):
/// - If (i + offset) < vlmax: vd[i] = vs2[i + offset].
/// - Otherwise: vd[i] = 0.
///
/// Elements in [vl, vlmax) follow the tail policy.  Masking applies normally.
fn exec_slidedown(
    vpr: &mut Vpr,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let offset = offset_from_operand(operand1);

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }

        let src_idx = i + offset;
        let val =
            if src_idx < vlmax { vpr.read_element(vs2, ElemIdx::new(src_idx), ctx.sew) } else { 0 };
        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
    }

    no_flags_result(None)
}

/// `vslide1up` — slide up by one, inserting a scalar at element 0.
///
/// - vd[0] = scalar from operand1 (rs1).
/// - vd[i] = vs2[i - 1] for i in 1..vl.
///
/// Elements in [vl, vlmax) follow the tail policy.  Masking applies normally.
fn exec_slide1up(
    vpr: &mut Vpr,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let scalar = scalar_from_operand(operand1, ctx.sew);

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }

        let val = if i == 0 { scalar } else { vpr.read_element(vs2, ElemIdx::new(i - 1), ctx.sew) };
        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
    }

    no_flags_result(None)
}

/// `vslide1down` — slide down by one, inserting a scalar at the last active
/// element.
///
/// - vd[i] = vs2[i + 1] for i in 0..vl-1.
/// - vd[vl - 1] = scalar from operand1 (rs1).
///
/// Elements in [vl, vlmax) follow the tail policy.  Masking applies normally.
fn exec_slide1down(
    vpr: &mut Vpr,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let scalar = scalar_from_operand(operand1, ctx.sew);

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }

        let val = if i == ctx.vl - 1 {
            scalar
        } else {
            vpr.read_element(vs2, ElemIdx::new(i + 1), ctx.sew)
        };
        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
    }

    no_flags_result(None)
}

// ============================================================================
// Gather
// ============================================================================

/// `vrgather` — register gather (permute by index).
///
/// For each active element i in [vstart, vl):
/// - Read the index from operand1 (vector, scalar, or immediate).
/// - If index >= vlmax, write 0.
/// - Otherwise, write vs2[index].
///
/// Elements in [vl, vlmax) follow the tail policy.  Masking applies normally.
fn exec_rgather(
    vpr: &mut Vpr,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }

        let index = match operand1 {
            VecOperand::Vector(vs1) => vpr.read_element(*vs1, ElemIdx::new(i), ctx.sew) as usize,
            VecOperand::Scalar(v) => *v as usize,
            VecOperand::Immediate(v) => *v as u64 as usize,
        };

        let val =
            if index >= vlmax { 0 } else { vpr.read_element(vs2, ElemIdx::new(index), ctx.sew) };
        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
    }

    no_flags_result(None)
}

/// `vrgatherei16` — register gather with 16-bit index vector.
///
/// Same as [`exec_rgather`] but indices from vs1 are always read at
/// [`Sew::E16`] regardless of the current SEW.  Data from vs2 is read at the
/// current SEW.
fn exec_rgather_ei16(
    vpr: &mut Vpr,
    vd: VRegIdx,
    vs2: VRegIdx,
    operand1: &VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    // operand1 must be a vector register for vrgatherei16.
    let vs1 = match operand1 {
        VecOperand::Vector(v) => *v,
        _ => unreachable!("vrgatherei16 requires vector operand for indices"),
    };

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if matches!(ctx.vta, TailPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if matches!(ctx.vma, MaskPolicy::Agnostic) {
                write_ones(vpr, vd, i, ctx.sew);
            }
            continue;
        }

        // Indices are always at EEW=16, regardless of current SEW.
        let index = vpr.read_element(vs1, ElemIdx::new(i), Sew::E16) as usize;

        let val =
            if index >= vlmax { 0 } else { vpr.read_element(vs2, ElemIdx::new(index), ctx.sew) };
        vpr.write_element(vd, ElemIdx::new(i), ctx.sew, val);
    }

    no_flags_result(None)
}

// ============================================================================
// Compress
// ============================================================================

/// `vcompress` — compress active elements from vs2 into vd.
///
/// Scans vs2 using the v0 mask: elements where the corresponding v0 bit is set
/// are packed contiguously into vd starting from element 0.  The operation is
/// always unmasked (vm=1); the mask register v0 selects which source elements
/// to include, not which destination elements to write.
///
/// Tail elements (after the last compressed element) follow the tail policy.
fn exec_compress(vpr: &mut Vpr, vd: VRegIdx, vs2: VRegIdx, ctx: &VecExecCtx) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut dst = 0usize;

    // Phase 1: pack active elements.
    for i in ctx.vstart..ctx.vl {
        if mask_active(vpr, i) {
            let val = vpr.read_element(vs2, ElemIdx::new(i), ctx.sew);
            vpr.write_element(vd, ElemIdx::new(dst), ctx.sew, val);
            dst += 1;
        }
    }

    // Phase 2: tail policy for remaining elements [dst, vlmax).
    if matches!(ctx.vta, TailPolicy::Agnostic) {
        for i in dst..vlmax {
            write_ones(vpr, vd, i, ctx.sew);
        }
    }

    no_flags_result(None)
}

// ============================================================================
// Whole-register moves
// ============================================================================

/// `vmv<n>r` — whole-register move of `n` consecutive registers.
///
/// Copies raw register bytes from vs2..vs2+(n-1) to vd..vd+(n-1).  Ignores
/// `vl`, `vtype`, masking, and tail policy — this is a raw byte copy.
fn exec_whole_reg_move(vpr: &mut Vpr, vd: VRegIdx, vs2: VRegIdx, nregs: u8) -> VecExecResult {
    for offset in 0..nregs {
        let src = VRegIdx::new(vs2.as_u8() + offset);
        let dst = VRegIdx::new(vd.as_u8() + offset);
        vpr.copy_reg(dst, src);
    }
    no_flags_result(None)
}
