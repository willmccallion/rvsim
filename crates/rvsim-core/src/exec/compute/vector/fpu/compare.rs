//! Floating-point comparisons into mask registers.

use crate::exec::compute::fpu::compare;
use crate::exec::compute::fpu::float::{Float, SoftF16};
use crate::exec::compute::vector::context::FillTarget;
use crate::exec::compute::vector::context::{
    FpSew, VecExecCtx, VecExecResult, VecOperand, read_op1,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::FpFlags;
use crate::isa::op::FpCompareOp;
use crate::isa::rvv::{ElemIdx, VRegIdx};

/// FP comparison loop: writes mask bits to vd.
///
/// Mask-producing instructions write one bit per element. The tail comprises
/// bits `[vl, VLEN)` in the destination mask register (RVV 1.0 §3.4.3), so
/// the loop must iterate over all VLEN mask bits, not just VLMAX elements.
pub(super) fn exec_fp_comparison(
    op: FpCompareOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match FpSew::of(ctx.sew, ctx.zvfh) {
        Some(FpSew::F16) => compare_loop::<SoftF16>(op, vpr, (vd_idx, vs2_idx), operand1, ctx),
        Some(FpSew::F32) => compare_loop::<f32>(op, vpr, (vd_idx, vs2_idx), operand1, ctx),
        Some(FpSew::F64) => compare_loop::<f64>(op, vpr, (vd_idx, vs2_idx), operand1, ctx),
        None => VecExecResult::default(),
    }
}

fn compare_loop<F: Float>(
    op: FpCompareOp,
    vpr: &mut impl VectorRegFile,
    (vd_idx, vs2_idx): (VRegIdx, VRegIdx),
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Mask registers hold VLEN bits; the tail extends from vl to VLEN-1.
    let vlen_bits = vpr.vlen().bits();
    let mut flags = FpFlags::NONE;

    for i in 0..vlen_bits {
        if ctx.skip_element(vpr, i, FillTarget::MaskBit(vd_idx)) {
            continue;
        }

        let vs2 = F::from_raw_bits(vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew));
        let op1 = F::from_raw_bits(read_op1(vpr, &operand1, i, ctx.sew));
        let (result, f) = compare(op, vs2, op1);
        flags = flags | f;
        vpr.write_mask_bit(vd_idx, ElemIdx::new(i), result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}
