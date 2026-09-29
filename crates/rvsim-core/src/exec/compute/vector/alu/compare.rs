//! Integer comparisons and add/subtract-with-carry into mask registers.

use crate::exec::compute::vector::alu::is_mask_producing_carry;
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, mask_active, read_op1, sign_extend,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax};

/// Evaluate a comparison for one element, returning a bool for the mask bit.
#[inline]
pub(super) fn compute_compare(op: VectorOp, vs2: u64, op1: u64, sew: Sew) -> bool {
    let s2 = sign_extend(vs2, sew);
    let s1 = sign_extend(op1, sew);
    match op {
        VectorOp::VMSeq => vs2 == op1,
        VectorOp::VMSne => vs2 != op1,
        VectorOp::VMSltu => vs2 < op1,
        VectorOp::VMSlt => s2 < s1,
        VectorOp::VMSleu => vs2 <= op1,
        VectorOp::VMSle => s2 <= s1,
        VectorOp::VMSgtu => vs2 > op1,
        VectorOp::VMSgt => s2 > s1,
        _ => unreachable!(),
    }
}

/// Comparison loop: writes mask bits to vd.
///
/// Mask-producing instructions write one bit per element. The tail comprises
/// bits `[vl, VLEN)` in the destination mask register (RVV 1.0 §3.4.3), so
/// the loop must iterate over all VLEN mask bits, not just VLMAX elements.
pub(super) fn exec_comparison(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Mask registers hold VLEN bits; the tail extends from vl to VLEN-1.
    let vlen_bits = vpr.vlen().bits();

    for i in 0..vlen_bits {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_mask_bit(vd_idx, ElemIdx::new(i), true);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_mask_bit(vd_idx, ElemIdx::new(i), true);
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let result = compute_compare(op, vs2_val, op1_val, ctx.sew);
        vpr.write_mask_bit(vd_idx, ElemIdx::new(i), result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}

/// Add/subtract with carry loop.
///
/// For mask-producing variants (`vmadc`, `vmsbc`), the tail comprises bits
/// `[vl, VLEN)` in the destination mask register (RVV 1.0 §3.4.3). For
/// non-mask variants (`vadc`, `vsbc`), the tail is `[vl, VLMAX)` at SEW.
/// We iterate to the larger of the two bounds so both cases are covered.
pub(super) fn exec_carry(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let vlen_bits = vpr.vlen().bits();
    let mask = ctx.sew.mask();
    let writes_mask = is_mask_producing_carry(op);
    // Mask-producing ops need tail up to VLEN bits; element-producing up to VLMAX.
    let loop_end = if writes_mask { vlen_bits } else { vlmax };

    for i in 0..loop_end {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                if writes_mask {
                    vpr.write_mask_bit(vd_idx, ElemIdx::new(i), true);
                } else {
                    vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
                }
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        // Carry/borrow comes from v0 mask. For vmadc/vmsbc with vm=1,
        // carry is 0 (no carry input).
        let carry = if ctx.vm { 0u64 } else { mask_active(vpr, i) as u64 };

        match op {
            VectorOp::VAdc => {
                let result = vs2_val.wrapping_add(op1_val).wrapping_add(carry) & mask;
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
            }
            VectorOp::VMadc => {
                let sum = (vs2_val as u128) + (op1_val as u128) + (carry as u128);
                let cout = sum > mask as u128;
                vpr.write_mask_bit(vd_idx, ElemIdx::new(i), cout);
            }
            VectorOp::VSbc => {
                let result = vs2_val.wrapping_sub(op1_val).wrapping_sub(carry) & mask;
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
            }
            VectorOp::VMsbc => {
                let borrow = (vs2_val as u128) < (op1_val as u128) + (carry as u128);
                vpr.write_mask_bit(vd_idx, ElemIdx::new(i), borrow);
            }
            _ => unreachable!(),
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: crate::isa::fp::FpFlags::NONE }
}
