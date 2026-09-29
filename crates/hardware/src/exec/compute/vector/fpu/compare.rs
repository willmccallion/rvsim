//! Floating-point comparisons into mask registers.

use super::{elem_to_f32, elem_to_f64};
use crate::exec::compute::fpu::half::{f16_to_f32, is_snan_f16};
use crate::exec::compute::fpu::nan_handling::{is_snan_f32, is_snan_f64};
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, mask_active, read_op1,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::FpFlags;
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx};

/// FP comparison loop: writes mask bits to vd.
///
/// Mask-producing instructions write one bit per element. The tail comprises
/// bits `[vl, VLEN)` in the destination mask register (RVV 1.0 §3.4.3), so
/// the loop must iterate over all VLEN mask bits, not just VLMAX elements.
pub(super) fn exec_fp_comparison(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    // Mask registers hold VLEN bits; the tail extends from vl to VLEN-1.
    let vlen_bits = vpr.vlen().bits();
    let mut flags = FpFlags::NONE;

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

        let (result, f) = match ctx.sew {
            Sew::E32 => {
                let a = elem_to_f32(vs2_val);
                let b = elem_to_f32(op1_val);
                // NaN comparisons: FLT/FLE/FGT/FGE raise NV on any NaN;
                // FEQ/FNE raise NV only on sNaN
                let nv = match op {
                    VectorOp::VMFEq | VectorOp::VMFNe => is_snan_f32(a) || is_snan_f32(b),
                    _ => a.is_nan() || b.is_nan(),
                };
                let f = if nv { FpFlags::NV } else { FpFlags::NONE };
                let cmp = match op {
                    VectorOp::VMFEq => a == b,
                    VectorOp::VMFNe => a != b,
                    VectorOp::VMFLt => a < b,
                    VectorOp::VMFLe => a <= b,
                    VectorOp::VMFGt => a > b,
                    VectorOp::VMFGe => a >= b,
                    _ => false,
                };
                (cmp, f)
            }
            Sew::E64 => {
                let a = elem_to_f64(vs2_val);
                let b = elem_to_f64(op1_val);
                let nv = match op {
                    VectorOp::VMFEq | VectorOp::VMFNe => is_snan_f64(a) || is_snan_f64(b),
                    _ => a.is_nan() || b.is_nan(),
                };
                let f = if nv { FpFlags::NV } else { FpFlags::NONE };
                let cmp = match op {
                    VectorOp::VMFEq => a == b,
                    VectorOp::VMFNe => a != b,
                    VectorOp::VMFLt => a < b,
                    VectorOp::VMFLe => a <= b,
                    VectorOp::VMFGt => a > b,
                    VectorOp::VMFGe => a >= b,
                    _ => false,
                };
                (cmp, f)
            }
            Sew::E16 if ctx.zvfh => {
                let a16 = vs2_val as u16;
                let b16 = op1_val as u16;
                let a = f16_to_f32(a16);
                let b = f16_to_f32(b16);
                let nv = match op {
                    VectorOp::VMFEq | VectorOp::VMFNe => is_snan_f16(a16) || is_snan_f16(b16),
                    _ => a.is_nan() || b.is_nan(),
                };
                let f = if nv { FpFlags::NV } else { FpFlags::NONE };
                let cmp = match op {
                    VectorOp::VMFEq => a == b,
                    VectorOp::VMFNe => a != b,
                    VectorOp::VMFLt => a < b,
                    VectorOp::VMFLe => a <= b,
                    VectorOp::VMFGt => a > b,
                    VectorOp::VMFGe => a >= b,
                    _ => false,
                };
                (cmp, f)
            }
            _ => (false, FpFlags::NONE),
        };

        flags = flags | f;
        vpr.write_mask_bit(vd_idx, ElemIdx::new(i), result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}
