//! Vector Integer ALU.
//!
//! Implements all RISC-V Vector Extension (RVV 1.0) integer arithmetic
//! operations. The main entry point [`vec_execute`] dispatches to per-element
//! loops that handle masking, prestart/tail policy, and the arithmetic itself.
//!
//! Operations are grouped into categories:
//! - Standard arithmetic: add, sub, rsub, and, or, xor, shifts, min/max
//! - Comparisons (write mask): seq, sne, slt, sle, sgt, etc.
//! - Add/subtract with carry: adc, sbc, madc, msbc
//! - Multiply / multiply-accumulate
//! - Division / remainder
//! - Widening arithmetic and multiply
//! - Narrowing shifts and clips
//! - Saturating and averaging arithmetic
//! - Fixed-point scaling: smul, ssrl, ssra
//! - Extension: zero/sign-extend at various ratios

mod compare;
mod integer;
mod widen;

use crate::exec::compute::vector::alu::compare::{exec_carry, exec_comparison};
use crate::exec::compute::vector::alu::integer::{
    exec_extension, exec_macc, exec_merge, exec_standard,
};
use crate::exec::compute::vector::alu::widen::{exec_narrowing, exec_widening, exec_widening_macc};
use crate::exec::compute::vector::context::{VecExecCtx, VecExecResult, VecOperand};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::RoundingMode;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{MaskPolicy, Sew, TailPolicy, VRegIdx, Vlmul, Vxrm};

/// Fractional SEW for extension operations (divide by given factor).
#[inline]
const fn frac_sew(sew: Sew, factor: usize) -> Option<Sew> {
    let target = sew.bits() / factor;
    match target {
        8 => Some(Sew::E8),
        16 => Some(Sew::E16),
        32 => Some(Sew::E32),
        64 => Some(Sew::E64),
        _ => None,
    }
}

#[inline]
const fn is_comparison(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VMSeq
            | VectorOp::VMSne
            | VectorOp::VMSltu
            | VectorOp::VMSlt
            | VectorOp::VMSleu
            | VectorOp::VMSle
            | VectorOp::VMSgtu
            | VectorOp::VMSgt
    )
}

#[inline]
const fn is_carry_op(op: VectorOp) -> bool {
    matches!(op, VectorOp::VAdc | VectorOp::VMadc | VectorOp::VSbc | VectorOp::VMsbc)
}

#[inline]
const fn is_mask_producing_carry(op: VectorOp) -> bool {
    matches!(op, VectorOp::VMadc | VectorOp::VMsbc)
}

#[inline]
const fn is_widening(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VWAddU
            | VectorOp::VWAdd
            | VectorOp::VWSubU
            | VectorOp::VWSub
            | VectorOp::VWAddUW
            | VectorOp::VWAddW
            | VectorOp::VWSubUW
            | VectorOp::VWSubW
            | VectorOp::VWMulU
            | VectorOp::VWMul
            | VectorOp::VWMulSU
            | VectorOp::VWsll
    )
}

#[inline]
const fn is_widening_macc(op: VectorOp) -> bool {
    matches!(op, VectorOp::VWMaccU | VectorOp::VWMacc | VectorOp::VWMaccSU | VectorOp::VWMaccUS)
}

/// `.w` variants read vs2 at the wide (2*SEW) width.
#[inline]
const fn is_wide_vs2(op: VectorOp) -> bool {
    matches!(op, VectorOp::VWAddUW | VectorOp::VWAddW | VectorOp::VWSubUW | VectorOp::VWSubW)
}

#[inline]
const fn is_narrowing(op: VectorOp) -> bool {
    matches!(op, VectorOp::VNSrl | VectorOp::VNSra | VectorOp::VNClipU | VectorOp::VNClip)
}

#[inline]
const fn is_macc(op: VectorOp) -> bool {
    matches!(op, VectorOp::VMacc | VectorOp::VNMSac | VectorOp::VMadd | VectorOp::VNMSub)
}

#[inline]
const fn is_extension(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::VZextVf2
            | VectorOp::VZextVf4
            | VectorOp::VZextVf8
            | VectorOp::VSextVf2
            | VectorOp::VSextVf4
            | VectorOp::VSextVf8
    )
}

/// Execute a vector integer ALU operation.
///
/// Iterates over all elements up to VLMAX, applying prestart/tail/mask
/// policies and computing the result for each active element. The destination
/// register group (`vd_idx`) is written in place.
///
/// # Arguments
///
/// * `op`       - The vector ALU operation to perform.
/// * `vpr`      - Mutable reference to the vector register file.
/// * `vd_idx`   - Destination vector register index.
/// * `vs2_idx`  - Second source vector register index.
/// * `operand1` - First source operand (vector, scalar, or immediate).
/// * `sew`      - Selected element width.
/// * `vl`       - Current vector length.
/// * `vstart`   - Current vstart value (elements before this are prestart).
/// * `vma`      - Masked-off element policy.
/// * `vta`      - Tail element policy.
/// * `vlmul`    - Vector length multiplier.
/// * `vm`       - Masking mode: `true` = unmasked, `false` = masked by v0.
/// * `vxrm`     - Fixed-point rounding mode.
#[allow(clippy::too_many_arguments)]
pub fn vec_execute(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    sew: Sew,
    vl: usize,
    vstart: usize,
    vma: MaskPolicy,
    vta: TailPolicy,
    vlmul: Vlmul,
    vm: bool,
    vxrm: Vxrm,
) -> VecExecResult {
    let ctx = VecExecCtx {
        sew,
        vl,
        vstart,
        vma,
        vta,
        vlmul,
        vm,
        vxrm,
        frm: RoundingMode::Rne,
        zvfh: false,
    };

    if is_comparison(op) {
        return exec_comparison(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if is_carry_op(op) {
        return exec_carry(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if is_widening(op) {
        return exec_widening(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if is_widening_macc(op) {
        return exec_widening_macc(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if is_narrowing(op) {
        return exec_narrowing(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if is_extension(op) {
        return exec_extension(op, vpr, vd_idx, vs2_idx, &ctx);
    }
    if is_macc(op) {
        return exec_macc(op, vpr, vd_idx, vs2_idx, operand1, &ctx);
    }
    if op == VectorOp::VMerge {
        return exec_merge(vpr, vd_idx, vs2_idx, operand1, &ctx);
    }

    exec_standard(op, vpr, vd_idx, vs2_idx, operand1, &ctx)
}

#[cfg(test)]
mod tests;
