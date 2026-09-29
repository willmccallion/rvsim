//! Fused multiply-add.

use super::{elem_to_f32, elem_to_f64};
use crate::exec::compute::fpu::half::{f16_to_f32, f64_to_f16, is_snan_f16};
use crate::exec::compute::fpu::nan_handling::{box_f32_canon, canonicalize_f64_bits};
use crate::exec::compute::fpu::{clear_host_fp_flags, read_host_fp_flags};
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, mask_active, read_op1,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::{FpFlags, RoundingMode};
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax};

/// Compute FMA for f16 element (Zvfh).
pub(super) fn compute_fma_f16(
    op: VectorOp,
    vs2_bits: u64,
    op1_bits: u64,
    vd_bits: u64,
    rm: RoundingMode,
) -> (u64, FpFlags) {
    let ha = vs2_bits as u16;
    let hb = op1_bits as u16;
    let hc = vd_bits as u16;
    let vs2 = f16_to_f32(ha) as f64;
    let op1 = f16_to_f32(hb) as f64;
    let vd = f16_to_f32(hc) as f64;

    let nv = if is_snan_f16(ha) || is_snan_f16(hb) || is_snan_f16(hc) {
        FpFlags::NV
    } else {
        FpFlags::NONE
    };

    let r = match op {
        VectorOp::VFMacc => op1.mul_add(vs2, vd),
        VectorOp::VFNMacc => (-op1).mul_add(vs2, -vd),
        VectorOp::VFMSac => op1.mul_add(vs2, -vd),
        VectorOp::VFNMSac => (-op1).mul_add(vs2, vd),
        VectorOp::VFMAdd => op1.mul_add(vd, vs2),
        VectorOp::VFNMAdd => (-op1).mul_add(vd, -vs2),
        VectorOp::VFMSub => op1.mul_add(vd, -vs2),
        VectorOp::VFNMSub => (-op1).mul_add(vd, vs2),
        _ => 0.0,
    };

    let (bits, flags) = f64_to_f16(r, rm);
    (bits as u64, flags | nv)
}

/// FMA operations: vd is both source (accumulator) and destination.
pub(super) fn exec_fp_fma(
    op: VectorOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    for i in 0..vlmax {
        if i < ctx.vstart {
            continue;
        }
        if i >= ctx.vl {
            if ctx.vta.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }
        if !ctx.vm && !mask_active(vpr, i) {
            if ctx.vma.is_agnostic() {
                vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, ctx.sew.ones());
            }
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let vd_val = vpr.read_element(vd_idx, ElemIdx::new(i), ctx.sew);

        let (result, f) = match ctx.sew {
            Sew::E32 => compute_fma_f32(op, vs2_val, op1_val, vd_val),
            Sew::E64 => compute_fma_f64(op, vs2_val, op1_val, vd_val),
            Sew::E16 if ctx.zvfh => compute_fma_f16(op, vs2_val, op1_val, vd_val, ctx.frm),
            _ => (0, FpFlags::NONE),
        };
        flags = flags | f;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// Compute FMA for f32 element.
///
/// RVV FMA conventions:
/// - `vfmacc.vv`: vd[i] = vs1[i]*vs2[i] + vd[i]
/// - `vfnmacc.vv`: vd[i] = -(vs1[i]*vs2[i]) - vd[i]
/// - `vfmsac.vv`: vd[i] = vs1[i]*vs2[i] - vd[i]
/// - `vfnmsac.vv`: vd[i] = -(vs1[i]*vs2[i]) + vd[i]
/// - `vfmadd.vv`: vd[i] = vs1[i]*vd[i] + vs2[i]
/// - `vfnmadd.vv`: vd[i] = -(vs1[i]*vd[i]) - vs2[i]
/// - `vfmsub.vv`: vd[i] = vs1[i]*vd[i] - vs2[i]
/// - `vfnmsub.vv`: vd[i] = -(vs1[i]*vd[i]) + vs2[i]
pub(super) fn compute_fma_f32(
    op: VectorOp,
    vs2_bits: u64,
    op1_bits: u64,
    vd_bits: u64,
) -> (u64, FpFlags) {
    let vs2 = elem_to_f32(vs2_bits);
    let op1 = elem_to_f32(op1_bits);
    let vd = elem_to_f32(vd_bits);

    clear_host_fp_flags();
    let r =
        std::hint::black_box(match op {
            VectorOp::VFMacc => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vs2), std::hint::black_box(vd)),
            VectorOp::VFNMacc => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vs2), -std::hint::black_box(vd)),
            VectorOp::VFMSac => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vs2), -std::hint::black_box(vd)),
            VectorOp::VFNMSac => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vs2), std::hint::black_box(vd)),
            VectorOp::VFMAdd => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vd), std::hint::black_box(vs2)),
            VectorOp::VFNMAdd => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vd), -std::hint::black_box(vs2)),
            VectorOp::VFMSub => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vd), -std::hint::black_box(vs2)),
            VectorOp::VFNMSub => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vd), std::hint::black_box(vs2)),
            _ => 0.0,
        });
    (box_f32_canon(r), read_host_fp_flags())
}

/// Compute FMA for f64 element.
pub(super) fn compute_fma_f64(
    op: VectorOp,
    vs2_bits: u64,
    op1_bits: u64,
    vd_bits: u64,
) -> (u64, FpFlags) {
    let vs2 = elem_to_f64(vs2_bits);
    let op1 = elem_to_f64(op1_bits);
    let vd = elem_to_f64(vd_bits);

    clear_host_fp_flags();
    let r =
        std::hint::black_box(match op {
            VectorOp::VFMacc => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vs2), std::hint::black_box(vd)),
            VectorOp::VFNMacc => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vs2), -std::hint::black_box(vd)),
            VectorOp::VFMSac => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vs2), -std::hint::black_box(vd)),
            VectorOp::VFNMSac => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vs2), std::hint::black_box(vd)),
            VectorOp::VFMAdd => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vd), std::hint::black_box(vs2)),
            VectorOp::VFNMAdd => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vd), -std::hint::black_box(vs2)),
            VectorOp::VFMSub => std::hint::black_box(op1)
                .mul_add(std::hint::black_box(vd), -std::hint::black_box(vs2)),
            VectorOp::VFNMSub => (-std::hint::black_box(op1))
                .mul_add(std::hint::black_box(vd), std::hint::black_box(vs2)),
            _ => 0.0,
        });
    (canonicalize_f64_bits(r), read_host_fp_flags())
}
