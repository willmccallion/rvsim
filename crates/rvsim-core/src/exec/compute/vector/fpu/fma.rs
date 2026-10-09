//! Fused multiply-add.

use super::{elem_to_f32, elem_to_f64};
use crate::exec::compute::fpu::exact::{self, Exact, Format, on_host_f32, on_host_f64};
use crate::exec::compute::fpu::half::{f16_to_f32, f64_to_f16, fused_is_invalid, is_snan_f16};
use crate::exec::compute::fpu::nan_handling::{box_f32_canon, canonicalize_f64_bits};
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, mask_active, read_op1,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::{FpFlags, RoundingMode};
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax};
use std::hint::black_box;

/// The multiplicands and addend of an FMA, `a × b + c`, as the RVV ops take
/// them from `vs1` (or the scalar), `vs2` and `vd`.
fn fma_operands<T: std::ops::Neg<Output = T> + Default>(
    op: VectorOp,
    op1: T,
    vs2: T,
    vd: T,
) -> (T, T, T) {
    match op {
        VectorOp::VFMacc => (op1, vs2, vd),
        VectorOp::VFNMacc => (-op1, vs2, -vd),
        VectorOp::VFMSac => (op1, vs2, -vd),
        VectorOp::VFNMSac => (-op1, vs2, vd),
        VectorOp::VFMAdd => (op1, vd, vs2),
        VectorOp::VFNMAdd => (-op1, vd, -vs2),
        VectorOp::VFMSub => (op1, vd, -vs2),
        VectorOp::VFNMSub => (-op1, vd, vs2),
        _ => (T::default(), T::default(), T::default()),
    }
}

/// Compute FMA for f16 element (Zvfh). The f64 fused result can be inexact
/// and then rounds twice, so an inexact one is rounded from the exact value.
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
    let (a, b, c) = fma_operands(
        op,
        f64::from(f16_to_f32(hb)),
        f64::from(f16_to_f32(ha)),
        f64::from(f16_to_f32(hc)),
    );
    let signaling_nan = is_snan_f16(ha) || is_snan_f16(hb) || is_snan_f16(hc);
    let nv = if signaling_nan || fused_is_invalid(a, b, c) { FpFlags::NV } else { FpFlags::NONE };

    let (bits, flags) = f64_to_f16(a.mul_add(b, c), rm);
    if flags.contains(FpFlags::NX) && a.is_finite() && b.is_finite() && c.is_finite() {
        let exact = exact::mul_add(Exact::of_f64(a), Exact::of_f64(b), Exact::of_f64(c));
        let (bits, flags) = exact::round(exact, Format::Half, rm);
        return (bits, flags | nv);
    }
    (u64::from(bits), flags | nv)
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

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
        let vd_val = vpr.read_element(vd_idx, ElemIdx::new(i), ctx.sew);

        let (result, f) = match ctx.sew {
            Sew::E32 => compute_fma_f32(op, vs2_val, op1_val, vd_val, ctx.frm),
            Sew::E64 => compute_fma_f64(op, vs2_val, op1_val, vd_val, ctx.frm),
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
    rm: RoundingMode,
) -> (u64, FpFlags) {
    let (a, b, c) =
        fma_operands(op, elem_to_f32(op1_bits), elem_to_f32(vs2_bits), elem_to_f32(vd_bits));
    let (r, flags) = on_host_f32(
        rm,
        || black_box(a).mul_add(black_box(b), black_box(c)),
        || exact::mul_add(Exact::of_f32(a), Exact::of_f32(b), Exact::of_f32(c)),
    );
    (box_f32_canon(r), flags)
}

/// Compute FMA for f64 element.
pub(super) fn compute_fma_f64(
    op: VectorOp,
    vs2_bits: u64,
    op1_bits: u64,
    vd_bits: u64,
    rm: RoundingMode,
) -> (u64, FpFlags) {
    let (a, b, c) =
        fma_operands(op, elem_to_f64(op1_bits), elem_to_f64(vs2_bits), elem_to_f64(vd_bits));
    let (r, flags) = on_host_f64(
        rm,
        || black_box(a).mul_add(black_box(b), black_box(c)),
        || exact::mul_add(Exact::of_f64(a), Exact::of_f64(b), Exact::of_f64(c)),
    );
    (canonicalize_f64_bits(r), flags)
}
