//! Single-width floating-point arithmetic, element by element.

use super::estimate::{vfrec7_16, vfrec7_32, vfrec7_64, vfrsqrt7_16, vfrsqrt7_32, vfrsqrt7_64};
use super::{F32_SIGN_BIT, F64_SIGN_BIT, elem_to_f32, elem_to_f64, float_to_int};
use crate::exec::compute::fpu::exact::{self, Exact, on_host_f32, on_host_f64};
use crate::exec::compute::fpu::half::{
    CANONICAL_NAN_F16, F16_SIGN, f16_to_f32, f64_to_f16, is_snan_f16, product_is_invalid,
    sum_is_invalid,
};
use crate::exec::compute::fpu::nan_handling::{
    box_f32, box_f32_canon, canonicalize_f64_bits, fmax_f32, fmax_f64, fmin_f32, fmin_f64,
};
use crate::exec::compute::fpu::nan_handling::{is_snan_f32, is_snan_f64};
use crate::exec::compute::vector::context::FillTarget;
use crate::exec::compute::vector::context::{VecExecCtx, VecExecResult, VecOperand, read_op1};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::{FpClass, FpFlags, FpFormat, RoundingMode};
use crate::isa::op::{FpBinaryOp, FpUnaryOp, IntConvert, Signedness};
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax};
use std::hint::black_box;

/// A single-width operation: one SEW result per element.
#[derive(Clone, Copy, Debug)]
pub(super) enum StandardOp {
    /// From `vs2` and `vs1` or `fs1`.
    Binary(FpBinaryOp),
    /// From `vs2` alone.
    Unary(FpUnaryOp),
}

pub(super) fn compute_f32(
    op: StandardOp,
    vs2_bits: u64,
    op1_bits: u64,
    frm: RoundingMode,
) -> (u64, FpFlags) {
    let a = elem_to_f32(vs2_bits);
    let b = elem_to_f32(op1_bits);

    match op {
        StandardOp::Binary(FpBinaryOp::Add) => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(a) + black_box(b),
                || exact::add(Exact::of_f32(a), Exact::of_f32(b)),
            );
            (box_f32_canon(r), flags)
        }
        StandardOp::Binary(FpBinaryOp::Sub) => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(a) - black_box(b),
                || exact::sub(Exact::of_f32(a), Exact::of_f32(b)),
            );
            (box_f32_canon(r), flags)
        }
        StandardOp::Binary(FpBinaryOp::RSub) => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(b) - black_box(a),
                || exact::sub(Exact::of_f32(b), Exact::of_f32(a)),
            );
            (box_f32_canon(r), flags)
        }
        StandardOp::Binary(FpBinaryOp::Mul) => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(a) * black_box(b),
                || exact::mul(Exact::of_f32(a), Exact::of_f32(b)),
            );
            (box_f32_canon(r), flags)
        }
        StandardOp::Binary(FpBinaryOp::Div) => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(a) / black_box(b),
                || exact::div(Exact::of_f32(a), Exact::of_f32(b)),
            );
            (box_f32_canon(r), flags)
        }
        StandardOp::Binary(FpBinaryOp::RDiv) => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(b) / black_box(a),
                || exact::div(Exact::of_f32(b), Exact::of_f32(a)),
            );
            (box_f32_canon(r), flags)
        }
        StandardOp::Unary(FpUnaryOp::Sqrt) => {
            let (r, flags) =
                on_host_f32(frm, || black_box(a).sqrt(), || exact::sqrt(Exact::of_f32(a)));
            (box_f32_canon(r), flags)
        }
        StandardOp::Unary(FpUnaryOp::Rsqrt7) => vfrsqrt7_32(vs2_bits as u32),
        StandardOp::Unary(FpUnaryOp::Rec7) => vfrec7_32(vs2_bits as u32, frm),
        StandardOp::Binary(FpBinaryOp::Min) => {
            let r = fmin_f32(a, b);
            // IEEE 754-2008 minNum: raise NV if either operand is a signaling NaN.
            let f = if is_snan_f32(a) || is_snan_f32(b) { FpFlags::NV } else { FpFlags::NONE };
            (box_f32(r), f)
        }
        StandardOp::Binary(FpBinaryOp::Max) => {
            let r = fmax_f32(a, b);
            let f = if is_snan_f32(a) || is_snan_f32(b) { FpFlags::NV } else { FpFlags::NONE };
            (box_f32(r), f)
        }
        StandardOp::Binary(FpBinaryOp::Sgnj) => {
            let r = f32::from_bits((a.to_bits() & !F32_SIGN_BIT) | (b.to_bits() & F32_SIGN_BIT));
            (box_f32(r), FpFlags::NONE)
        }
        StandardOp::Binary(FpBinaryOp::Sgnjn) => {
            let r = f32::from_bits((a.to_bits() & !F32_SIGN_BIT) | (!b.to_bits() & F32_SIGN_BIT));
            (box_f32(r), FpFlags::NONE)
        }
        StandardOp::Binary(FpBinaryOp::Sgnjx) => {
            let r = f32::from_bits(a.to_bits() ^ (b.to_bits() & F32_SIGN_BIT));
            (box_f32(r), FpFlags::NONE)
        }
        StandardOp::Unary(FpUnaryOp::Class) => {
            (FpClass::of(FpFormat::Single, vs2_bits).mask(), FpFlags::NONE)
        }
        StandardOp::Unary(FpUnaryOp::Convert(IntConvert::ToInt(signedness, rounding))) => {
            float_to_int(f64::from(a), (signedness, rounding), 32, frm)
        }
        // Conversions: unsigned int -> float
        StandardOp::Unary(FpUnaryOp::Convert(IntConvert::FromInt(Signedness::Unsigned))) => {
            let unsigned = vs2_bits as u32;
            let (r, flags) = on_host_f32(
                frm,
                || black_box(unsigned) as f32,
                || Exact::of_unsigned(u128::from(unsigned)),
            );
            (box_f32_canon(r), flags)
        }
        // Conversions: signed int -> float
        StandardOp::Unary(FpUnaryOp::Convert(IntConvert::FromInt(Signedness::Signed))) => {
            let signed = Sew::E32.sign_extend(vs2_bits) as i32;
            let (r, flags) = on_host_f32(
                frm,
                || black_box(signed) as f32,
                || Exact::of_integer(i128::from(signed)),
            );
            (box_f32_canon(r), flags)
        }
    }
}

/// Compute one standard FP element at SEW=64.
///
/// Returns `(result_bits, flags)`.
pub(super) fn compute_f64(
    op: StandardOp,
    vs2_bits: u64,
    op1_bits: u64,
    frm: RoundingMode,
) -> (u64, FpFlags) {
    let a = elem_to_f64(vs2_bits);
    let b = elem_to_f64(op1_bits);

    match op {
        StandardOp::Binary(FpBinaryOp::Add) => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(a) + black_box(b),
                || exact::add(Exact::of_f64(a), Exact::of_f64(b)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        StandardOp::Binary(FpBinaryOp::Sub) => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(a) - black_box(b),
                || exact::sub(Exact::of_f64(a), Exact::of_f64(b)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        StandardOp::Binary(FpBinaryOp::RSub) => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(b) - black_box(a),
                || exact::sub(Exact::of_f64(b), Exact::of_f64(a)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        StandardOp::Binary(FpBinaryOp::Mul) => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(a) * black_box(b),
                || exact::mul(Exact::of_f64(a), Exact::of_f64(b)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        StandardOp::Binary(FpBinaryOp::Div) => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(a) / black_box(b),
                || exact::div(Exact::of_f64(a), Exact::of_f64(b)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        StandardOp::Binary(FpBinaryOp::RDiv) => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(b) / black_box(a),
                || exact::div(Exact::of_f64(b), Exact::of_f64(a)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        StandardOp::Unary(FpUnaryOp::Sqrt) => {
            let (r, flags) =
                on_host_f64(frm, || black_box(a).sqrt(), || exact::sqrt(Exact::of_f64(a)));
            (canonicalize_f64_bits(r), flags)
        }
        StandardOp::Unary(FpUnaryOp::Rsqrt7) => vfrsqrt7_64(vs2_bits),
        StandardOp::Unary(FpUnaryOp::Rec7) => vfrec7_64(vs2_bits, frm),
        StandardOp::Binary(FpBinaryOp::Min) => {
            let r = fmin_f64(a, b);
            let f = if is_snan_f64(a) || is_snan_f64(b) { FpFlags::NV } else { FpFlags::NONE };
            (r.to_bits(), f)
        }
        StandardOp::Binary(FpBinaryOp::Max) => {
            let r = fmax_f64(a, b);
            let f = if is_snan_f64(a) || is_snan_f64(b) { FpFlags::NV } else { FpFlags::NONE };
            (r.to_bits(), f)
        }
        StandardOp::Binary(FpBinaryOp::Sgnj) => {
            let r = f64::from_bits((a.to_bits() & !F64_SIGN_BIT) | (b.to_bits() & F64_SIGN_BIT));
            (r.to_bits(), FpFlags::NONE)
        }
        StandardOp::Binary(FpBinaryOp::Sgnjn) => {
            let r = f64::from_bits((a.to_bits() & !F64_SIGN_BIT) | (!b.to_bits() & F64_SIGN_BIT));
            (r.to_bits(), FpFlags::NONE)
        }
        StandardOp::Binary(FpBinaryOp::Sgnjx) => {
            let r = f64::from_bits(a.to_bits() ^ (b.to_bits() & F64_SIGN_BIT));
            (r.to_bits(), FpFlags::NONE)
        }
        StandardOp::Unary(FpUnaryOp::Class) => {
            (FpClass::of(FpFormat::Double, vs2_bits).mask(), FpFlags::NONE)
        }
        StandardOp::Unary(FpUnaryOp::Convert(IntConvert::ToInt(signedness, rounding))) => {
            float_to_int(a, (signedness, rounding), 64, frm)
        }
        StandardOp::Unary(FpUnaryOp::Convert(IntConvert::FromInt(Signedness::Unsigned))) => {
            // Workaround for an LLVM codegen quirk: `0u64 as f64` under host
            // FE_DOWNWARD/FE_UPWARD produces a signed zero with the wrong
            // sign (the software fallback used when CVTUSI2SD_q is unavailable
            // doesn't short-circuit the zero case). Treat val=0 explicitly.
            let (r, flags) = on_host_f64(
                frm,
                || if vs2_bits == 0 { 0.0 } else { black_box(vs2_bits) as f64 },
                || Exact::of_unsigned(u128::from(vs2_bits)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        StandardOp::Unary(FpUnaryOp::Convert(IntConvert::FromInt(Signedness::Signed))) => {
            let signed = vs2_bits as i64;
            let (r, flags) = on_host_f64(
                frm,
                || black_box(signed) as f64,
                || Exact::of_integer(i128::from(signed)),
            );
            (canonicalize_f64_bits(r), flags)
        }
    }
}

/// Compute one standard FP element at SEW=16 (Zvfh).
///
/// Vector f16 values are stored as raw u16 bit patterns in the low 16 bits of
/// each element slot — they are NOT NaN-boxed (that is only for scalar registers).
/// Arithmetic is performed by upcasting to f64 (lossless for f16 inputs) and
/// software-rounding back to f16 via `f64_to_f16`.
pub(super) fn compute_f16(
    op: StandardOp,
    vs2_bits: u64,
    op1_bits: u64,
    rm: RoundingMode,
) -> (u64, FpFlags) {
    let ha = vs2_bits as u16;
    let hb = op1_bits as u16;
    let fa = f16_to_f32(ha) as f64;
    let fb = f16_to_f32(hb) as f64;

    // Helper: round f64 result → f16 bit pattern, merging extra flags.
    let round = |val: f64, extra: FpFlags| -> (u64, FpFlags) {
        let (bits, flags) = f64_to_f16(val, rm);
        (bits as u64, flags | extra)
    };
    let signaling_nan = is_snan_f16(ha) || is_snan_f16(hb);
    let invalid = |invalid_operation: bool| {
        if signaling_nan || invalid_operation { FpFlags::NV } else { FpFlags::NONE }
    };

    match op {
        StandardOp::Binary(FpBinaryOp::Add) => round(fa + fb, invalid(sum_is_invalid(fa, fb))),
        StandardOp::Binary(FpBinaryOp::Sub) => round(fa - fb, invalid(sum_is_invalid(fa, -fb))),
        StandardOp::Binary(FpBinaryOp::RSub) => round(fb - fa, invalid(sum_is_invalid(fb, -fa))),
        StandardOp::Binary(FpBinaryOp::Mul) => round(fa * fb, invalid(product_is_invalid(fa, fb))),
        StandardOp::Binary(FpBinaryOp::Div) => {
            let mut extra = FpFlags::NONE;
            if is_snan_f16(ha) || is_snan_f16(hb) {
                extra = extra | FpFlags::NV;
            } else if fb == 0.0 && fa != 0.0 && fa.is_finite() {
                extra = extra | FpFlags::DZ;
            } else if (fb == 0.0 && fa == 0.0) || (fa.is_infinite() && fb.is_infinite()) {
                // 0/0 and ∞/∞ are both invalid operations.
                extra = extra | FpFlags::NV;
            }
            round(fa / fb, extra)
        }
        StandardOp::Binary(FpBinaryOp::RDiv) => {
            let mut extra = FpFlags::NONE;
            if is_snan_f16(ha) || is_snan_f16(hb) {
                extra = extra | FpFlags::NV;
            } else if fa == 0.0 && fb != 0.0 && fb.is_finite() {
                extra = extra | FpFlags::DZ;
            } else if (fa == 0.0 && fb == 0.0) || (fb.is_infinite() && fa.is_infinite()) {
                // 0/0 and ∞/∞ are both invalid operations.
                extra = extra | FpFlags::NV;
            }
            round(fb / fa, extra)
        }
        StandardOp::Unary(FpUnaryOp::Sqrt) => {
            let mut extra = FpFlags::NONE;
            if is_snan_f16(ha) || fa < 0.0 {
                extra = extra | FpFlags::NV;
            }
            round(fa.sqrt(), extra)
        }
        StandardOp::Binary(FpBinaryOp::Min) => {
            let f = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            let fa32 = f16_to_f32(ha);
            let fb32 = f16_to_f32(hb);
            let r = fmin_f32(fa32, fb32);
            if r.is_nan() {
                return (CANONICAL_NAN_F16 as u64, f);
            }
            let (bits, _) = f64_to_f16(r as f64, RoundingMode::Rne);
            (bits as u64, f)
        }
        StandardOp::Binary(FpBinaryOp::Max) => {
            let f = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            let fa32 = f16_to_f32(ha);
            let fb32 = f16_to_f32(hb);
            let r = fmax_f32(fa32, fb32);
            if r.is_nan() {
                return (CANONICAL_NAN_F16 as u64, f);
            }
            let (bits, _) = f64_to_f16(r as f64, RoundingMode::Rne);
            (bits as u64, f)
        }
        StandardOp::Binary(FpBinaryOp::Sgnj) => {
            (u64::from((ha & !F16_SIGN) | (hb & F16_SIGN)), FpFlags::NONE)
        }
        StandardOp::Binary(FpBinaryOp::Sgnjn) => {
            (u64::from((ha & !F16_SIGN) | (!hb & F16_SIGN)), FpFlags::NONE)
        }
        StandardOp::Binary(FpBinaryOp::Sgnjx) => (u64::from(ha ^ (hb & F16_SIGN)), FpFlags::NONE),
        StandardOp::Unary(FpUnaryOp::Class) => {
            (FpClass::of(FpFormat::Half, u64::from(ha)).mask(), FpFlags::NONE)
        }
        StandardOp::Unary(FpUnaryOp::Convert(IntConvert::ToInt(signedness, rounding))) => {
            float_to_int(fa, (signedness, rounding), 16, rm)
        }
        StandardOp::Unary(FpUnaryOp::Convert(IntConvert::FromInt(Signedness::Unsigned))) => {
            round(vs2_bits as u16 as f64, FpFlags::NONE)
        }
        StandardOp::Unary(FpUnaryOp::Convert(IntConvert::FromInt(Signedness::Signed))) => {
            round(Sew::E16.sign_extend(vs2_bits) as i16 as f64, FpFlags::NONE)
        }
        StandardOp::Unary(FpUnaryOp::Rsqrt7) => {
            let (r, f) = vfrsqrt7_16(ha);
            (r as u64, f)
        }
        StandardOp::Unary(FpUnaryOp::Rec7) => {
            let (r, f) = vfrec7_16(ha, rm);
            (r as u64, f)
        }
    }
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
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    for i in 0..vlmax {
        if ctx.skip_element(vpr, i, FillTarget::Element(vd_idx, ctx.sew)) {
            continue;
        }

        let vs2_val = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1_val = read_op1(vpr, &operand1, i, ctx.sew);

        let (result, f) = match ctx.sew {
            Sew::E32 => compute_f32(op, vs2_val, op1_val, ctx.frm),
            Sew::E64 => compute_f64(op, vs2_val, op1_val, ctx.frm),
            Sew::E16 if ctx.zvfh => compute_f16(op, vs2_val, op1_val, ctx.frm),
            _ => (0, FpFlags::NONE),
        };
        flags = flags | f;
        vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}
