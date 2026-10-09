//! Merges and one-element slides of floating-point scalars.

use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, mask_active, read_op1,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::FpFlags;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, VRegIdx, Vlmax};

/// FP merge: like integer merge but for FP values.
pub(super) fn exec_fp_merge(
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.fill_agnostic_element(vd_idx, ElemIdx::new(i), ctx.sew);
            }
            continue;
        }

        let use_op1 = ctx.vm || mask_active(vpr, i);
        let result = if use_op1 {
            read_op1(vpr, &operand1, i, ctx.sew)
        } else {
            vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew)
        };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE }
}

/// FP slide1up/slide1down operations.
pub(super) fn exec_fp_slide1(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let scalar = match operand1 {
        VecOperand::Scalar(s) => s & ctx.sew.mask(),
        _ => 0,
    };

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.fill_agnostic_element(vd_idx, ElemIdx::new(i), ctx.sew);
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.fill_agnostic_element(vd_idx, ElemIdx::new(i), ctx.sew);
            }
            continue;
        }

        let result = match op {
            VectorOp::VFSlide1Up => {
                if i == 0 {
                    scalar
                } else {
                    vpr.read_element(vs2_idx, ElemIdx::new(i - 1), ctx.sew)
                }
            }
            VectorOp::VFSlide1Down => {
                if i == ctx.vl - 1 {
                    scalar
                } else {
                    vpr.read_element(vs2_idx, ElemIdx::new(i + 1), ctx.sew)
                }
            }
            _ => 0,
        };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: FpFlags::NONE }
}
