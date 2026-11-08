//! Single-width floating-point arithmetic, element by element.

use super::float::VecFloat;
use super::float_to_int;
use crate::exec::compute::fpu::float::SoftF16;
use crate::exec::compute::fpu::{invalid_if_signaling, sign_inject};
use crate::exec::compute::vector::context::FillTarget;
use crate::exec::compute::vector::context::{
    FpSew, VecExecCtx, VecExecResult, VecOperand, read_op1,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::{FpClass, FpFlags, RoundingMode};
use crate::isa::op::{FpBinaryOp, FpUnaryOp, IntConvert, SignInjectOp, Signedness};
use crate::isa::rvv::{ElemIdx, VRegIdx, Vlmax};

/// A single-width operation: one SEW result per element.
#[derive(Clone, Copy, Debug)]
pub(super) enum StandardOp {
    /// From `vs2` and `vs1` or `fs1`.
    Binary(FpBinaryOp),
    /// From `vs2` alone.
    Unary(FpUnaryOp),
}

/// Standard (non-widening, non-narrowing, non-FMA) FP element-wise loop.
pub(super) fn exec_fp_standard(
    op: StandardOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    match FpSew::of(ctx.sew, ctx.zvfh) {
        Some(FpSew::F16) => standard_loop::<SoftF16>(op, vpr, (vd_idx, vs2_idx), operand1, ctx),
        Some(FpSew::F32) => standard_loop::<f32>(op, vpr, (vd_idx, vs2_idx), operand1, ctx),
        Some(FpSew::F64) => standard_loop::<f64>(op, vpr, (vd_idx, vs2_idx), operand1, ctx),
        None => VecExecResult::default(),
    }
}

fn standard_loop<F: VecFloat>(
    op: StandardOp,
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

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let (result, f) = match op {
            StandardOp::Binary(op) => {
                let op1_val = read_op1(vpr, &operand1, i, ctx.sew);
                binary(op, F::from_raw_bits(vs2_val), F::from_raw_bits(op1_val), ctx.frm)
            }
            StandardOp::Unary(op) => unary::<F>(op, vs2_val, ctx.frm),
        };
        flags = flags | f;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// One element of a binary operation on `vs2` (`a`) and `op1` (`b`).
fn binary<F: VecFloat>(op: FpBinaryOp, a: F, b: F, frm: RoundingMode) -> (u64, FpFlags) {
    let canonical = |(result, flags): (F, FpFlags)| (result.canonical().raw_bits(), flags);
    let injected = |op| (sign_inject(op, a, b).raw_bits(), FpFlags::NONE);
    match op {
        FpBinaryOp::Add => canonical(a.add(b, frm)),
        FpBinaryOp::Sub => canonical(a.sub(b, frm)),
        FpBinaryOp::RSub => canonical(b.sub(a, frm)),
        FpBinaryOp::Mul => canonical(a.mul(b, frm)),
        FpBinaryOp::Div => canonical(a.div(b, frm)),
        FpBinaryOp::RDiv => canonical(b.div(a, frm)),
        FpBinaryOp::Min => (a.min(b).raw_bits(), invalid_if_signaling(a, b)),
        FpBinaryOp::Max => (a.max(b).raw_bits(), invalid_if_signaling(a, b)),
        FpBinaryOp::Sgnj => injected(SignInjectOp::Copy),
        FpBinaryOp::Sgnjn => injected(SignInjectOp::Negate),
        FpBinaryOp::Sgnjx => injected(SignInjectOp::Xor),
    }
}

/// One element of a unary operation on the `vs2` element `bits`.
fn unary<F: VecFloat>(op: FpUnaryOp, bits: u64, frm: RoundingMode) -> (u64, FpFlags) {
    let value = F::from_raw_bits(bits);
    let canonical = |(result, flags): (F, FpFlags)| (result.canonical().raw_bits(), flags);
    match op {
        FpUnaryOp::Sqrt => canonical(value.sqrt(frm)),
        FpUnaryOp::Rsqrt7 => F::rsqrt7(bits),
        FpUnaryOp::Rec7 => F::rec7(bits, frm),
        FpUnaryOp::Class => (FpClass::of(F::FORMAT, bits).mask(), FpFlags::NONE),
        FpUnaryOp::Convert(IntConvert::ToInt(signedness, rounding)) => {
            float_to_int(value.to_f64(), (signedness, rounding), F::SEW.bits() as u32, frm)
        }
        FpUnaryOp::Convert(IntConvert::FromInt(Signedness::Unsigned)) => {
            canonical(F::from_int(bits & F::SEW.mask(), (F::INT_WIDTH, Signedness::Unsigned), frm))
        }
        FpUnaryOp::Convert(IntConvert::FromInt(Signedness::Signed)) => {
            let value = F::SEW.sign_extend(bits) as u64;
            canonical(F::from_int(value, (F::INT_WIDTH, Signedness::Signed), frm))
        }
    }
}
