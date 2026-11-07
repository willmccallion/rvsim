//! Fused multiply-add.
//!
//! RVV FMA conventions:
//! - `vfmacc.vv`: vd[i] = vs1[i]*vs2[i] + vd[i]
//! - `vfnmacc.vv`: vd[i] = -(vs1[i]*vs2[i]) - vd[i]
//! - `vfmsac.vv`: vd[i] = vs1[i]*vs2[i] - vd[i]
//! - `vfnmsac.vv`: vd[i] = -(vs1[i]*vs2[i]) + vd[i]
//! - `vfmadd.vv`: vd[i] = vs1[i]*vd[i] + vs2[i]
//! - `vfnmadd.vv`: vd[i] = -(vs1[i]*vd[i]) - vs2[i]
//! - `vfmsub.vv`: vd[i] = vs1[i]*vd[i] - vs2[i]
//! - `vfnmsub.vv`: vd[i] = -(vs1[i]*vd[i]) + vs2[i]

use super::float::{SoftF16, VecFloat};
use crate::exec::compute::vector::context::FillTarget;
use crate::exec::compute::vector::context::{
    FpSew, VecExecCtx, VecExecResult, VecOperand, read_op1,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::FpFlags;
use crate::isa::op::{FmaAddend, FmaSigns};
use crate::isa::rvv::{ElemIdx, VRegIdx, Vlmax};

/// The multiplicands and addend of an FMA, `a × b + c`, as the RVV ops take
/// them from `vs1` (or the scalar), `vs2` and `vd`.
pub(super) fn fma_operands<T: std::ops::Neg<Output = T>>(
    (signs, addend): (FmaSigns, FmaAddend),
    op1: T,
    vs2: T,
    vd: T,
) -> (T, T, T) {
    let (multiplicand, added) = match addend {
        FmaAddend::Vd => (vs2, vd),
        FmaAddend::Vs2 => (vd, vs2),
    };
    match signs {
        FmaSigns::Macc => (op1, multiplicand, added),
        FmaSigns::NMacc => (-op1, multiplicand, -added),
        FmaSigns::MSac => (op1, multiplicand, -added),
        FmaSigns::NMSac => (-op1, multiplicand, added),
    }
}

/// FMA operations: vd is both source (accumulator) and destination.
pub(super) fn exec_fp_fma(
    form: (FmaSigns, FmaAddend),
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match FpSew::of(ctx.sew, ctx.zvfh) {
        Some(FpSew::F16) => fma_loop::<SoftF16>(form, vpr, (vd_idx, vs2_idx), operand1, ctx),
        Some(FpSew::F32) => fma_loop::<f32>(form, vpr, (vd_idx, vs2_idx), operand1, ctx),
        Some(FpSew::F64) => fma_loop::<f64>(form, vpr, (vd_idx, vs2_idx), operand1, ctx),
        None => VecExecResult::default(),
    }
}

fn fma_loop<F: VecFloat>(
    form: (FmaSigns, FmaAddend),
    vpr: &mut impl VectorRegFile,
    (vd_idx, vs2_idx): (VRegIdx, VRegIdx),
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    for i in 0..vlmax {
        if ctx.skip_element(vpr, i, FillTarget::Element(vd_idx, ctx.sew)) {
            continue;
        }

        let element = |bits: u64| F::from_elem(bits);
        let vs2 = element(vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew));
        let op1 = element(read_op1(vpr, &operand1, i, ctx.sew));
        let vd = element(vpr.read_element(vd_idx, ElemIdx::new(i), ctx.sew));

        let (a, b, c) = fma_operands(form, op1, vs2, vd);
        let (result, f) = a.mul_add(b, c, ctx.frm);
        flags = flags | f;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result.canonical_elem_bits());
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}
