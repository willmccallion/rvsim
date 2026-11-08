//! Merges and one-element slides of floating-point scalars.

use crate::exec::compute::vector::context::FillTarget;
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, mask_active, read_op1,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
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
        if ctx.skip_outside_body(vpr, i, FillTarget::Element(vd_idx, ctx.sew)) {
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

    VecExecResult::default()
}

/// Which way a one-element slide moves.
#[derive(Clone, Copy, Debug)]
pub(super) enum SlideDirection {
    /// `vfslide1up`: towards higher elements, the scalar entering element 0.
    Up,
    /// `vfslide1down`: towards lower elements, the scalar entering element
    /// `vl - 1`.
    Down,
}

/// FP slide1up/slide1down operations.
pub(super) fn exec_fp_slide1(
    direction: SlideDirection,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    rs1: u64,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let scalar = rs1 & ctx.sew.mask();

    for i in 0..vlmax {
        if ctx.skip_element(vpr, i, FillTarget::Element(vd_idx, ctx.sew)) {
            continue;
        }

        let result = match direction {
            SlideDirection::Up => {
                if i == 0 {
                    scalar
                } else {
                    vpr.read_element(vs2_idx, ElemIdx::new(i - 1), ctx.sew)
                }
            }
            SlideDirection::Down => {
                if i == ctx.vl - 1 {
                    scalar
                } else {
                    vpr.read_element(vs2_idx, ElemIdx::new(i + 1), ctx.sew)
                }
            }
        };
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult::default()
}
