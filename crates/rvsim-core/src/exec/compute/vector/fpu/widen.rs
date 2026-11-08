//! Widening and narrowing operations, including widening FMA.

use super::convert::f64_to_f32_round_to_odd;
use super::fma::fma_operands;
use super::{elem_to_f32, elem_to_f64, float_to_int};
use crate::exec::compute::fpu::exact::{Exact, on_host_f32};
use crate::exec::compute::fpu::float::{Float, SoftF16};
use crate::exec::compute::fpu::half::{f16_to_f32, f64_to_f16, is_snan_f16};
use crate::exec::compute::fpu::nan_handling::{box_f32_canon, canonicalize_f64_bits, is_snan_f32};
use crate::exec::compute::vector::context::FillTarget;
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, read_op1, widen_sew,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::bits::zext_word;
use crate::isa::fp::{FpFlags, RoundingMode};
use crate::isa::op::{
    FmaAddend, FmaSigns, FpNarrowConvertOp, FpWidenConvertOp, FpWidenOp, IntConvert, Signedness,
};
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax};
use std::hint::black_box;
use std::ops::Neg;

/// Widening FP arithmetic: read at SEW (2*SEW for the `.w` forms' `vs2`),
/// write at 2*SEW.
pub(super) fn exec_fp_widening(
    op: FpWidenOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let element: fn(FpWidenOp, u64, u64, RoundingMode) -> (u64, FpFlags) = match ctx.sew {
        Sew::E32 => widening_arith::<f32, f64>,
        Sew::E16 => widening_arith::<SoftF16, f32>,
        Sew::E8 | Sew::E64 => return VecExecResult::default(),
    };
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult::default();
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;
    let vs2_sew = if is_wide_vs2(op) { wsew } else { ctx.sew };

    for i in 0..vlmax {
        if ctx.skip_element(vpr, i, FillTarget::Element(vd_idx, wsew)) {
            continue;
        }
        let vs2 = vpr.read_element(vs2_idx, ElemIdx::new(i), vs2_sew);
        let op1 = read_op1(vpr, &operand1, i, ctx.sew);
        let (result, f) = element(op, vs2, op1, ctx.frm);
        flags = flags | f;
        vpr.write_element(vd_idx, ElemIdx::new(i), wsew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// Whether `op` is a `.w` form, whose `vs2` is already 2*SEW.
const fn is_wide_vs2(op: FpWidenOp) -> bool {
    matches!(op, FpWidenOp::AddW | FpWidenOp::SubW)
}

/// The `N` element `bits` widened exactly to `W`, and whether it was a
/// signaling NaN, which widening quiets.
fn widened<N: Float, W: Float>(bits: u64) -> (W, bool) {
    let narrow = N::from_raw_bits(bits);
    (W::from_f64_exact(narrow.to_f64()), narrow.is_signaling_nan())
}

/// One element of a widening add, subtract or multiply: `vs2` at `N` (at
/// `W` for the `.w` forms) and `op1` at `N`, computed and returned at `W`.
fn widening_arith<N: Float, W: Float>(
    op: FpWidenOp,
    vs2: u64,
    op1: u64,
    rm: RoundingMode,
) -> (u64, FpFlags) {
    let (vs2, vs2_signaling) = if is_wide_vs2(op) {
        let wide = W::from_raw_bits(vs2);
        (wide, wide.is_signaling_nan())
    } else {
        widened::<N, W>(vs2)
    };
    let (op1, op1_signaling) = widened::<N, W>(op1);
    let (result, flags) = match op {
        FpWidenOp::Add | FpWidenOp::AddW => vs2.add(op1, rm),
        FpWidenOp::Sub | FpWidenOp::SubW => vs2.sub(op1, rm),
        FpWidenOp::Mul => vs2.mul(op1, rm),
    };
    (result.canonical().raw_bits(), flags | invalid_if(vs2_signaling || op1_signaling))
}

/// Widening conversions: read `vs2` at SEW, write at 2*SEW.
pub(super) fn exec_fp_widen_convert(
    op: FpWidenConvertOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult::default();
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    for i in 0..vlmax {
        if ctx.skip_element(vpr, i, FillTarget::Element(vd_idx, wsew)) {
            continue;
        }

        let vs2_raw = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let converted = match ctx.sew {
            Sew::E32 => Some(widen_from_single(op, vs2_raw, ctx.frm)),
            Sew::E16 => Some(widen_from_half(op, vs2_raw, ctx.frm)),
            Sew::E8 => widen_from_byte(op, vs2_raw, ctx.frm),
            Sew::E64 => None,
        };
        if let Some((bits, f)) = converted {
            flags = flags | f;
            vpr.write_element(vd_idx, ElemIdx::new(i), wsew, bits);
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// A widening conversion of the 32-bit element `raw` to 64 bits.
fn widen_from_single(op: FpWidenConvertOp, raw: u64, frm: RoundingMode) -> (u64, FpFlags) {
    let value = elem_to_f32(raw);
    match op {
        FpWidenConvertOp::Int(IntConvert::ToInt(signedness, rounding)) => {
            float_to_int(f64::from(value), (signedness, rounding), 64, frm)
        }
        FpWidenConvertOp::Int(IntConvert::FromInt(Signedness::Unsigned)) => {
            (f64::from(raw as u32).to_bits(), FpFlags::NONE)
        }
        FpWidenConvertOp::Int(IntConvert::FromInt(Signedness::Signed)) => {
            (f64::from(Sew::E32.sign_extend(raw) as i32).to_bits(), FpFlags::NONE)
        }
        FpWidenConvertOp::Float => {
            (canonicalize_f64_bits(f64::from(value)), invalid_if(is_snan_f32(value)))
        }
    }
}

/// A widening conversion of the 16-bit element `raw` to 32 bits. f16
/// sources need Zvfh, which the caller checked; 16-bit integers convert to
/// f32 without it.
fn widen_from_half(op: FpWidenConvertOp, raw: u64, frm: RoundingMode) -> (u64, FpFlags) {
    let half = raw as u16;
    match op {
        FpWidenConvertOp::Int(IntConvert::ToInt(signedness, rounding)) => {
            float_to_int(f64::from(f16_to_f32(half)), (signedness, rounding), 32, frm)
        }
        FpWidenConvertOp::Int(IntConvert::FromInt(Signedness::Unsigned)) => {
            (u64::from(f32::from(half).to_bits()), FpFlags::NONE)
        }
        FpWidenConvertOp::Int(IntConvert::FromInt(Signedness::Signed)) => {
            let signed = Sew::E16.sign_extend(raw) as i16;
            (u64::from(f32::from(signed).to_bits()), FpFlags::NONE)
        }
        FpWidenConvertOp::Float => (box_f32_canon(f16_to_f32(half)), invalid_if(is_snan_f16(half))),
    }
}

/// A widening conversion of the 8-bit integer `raw` to f16 (Zvfh), which
/// holds every one exactly; `None` for the conversions from floating
/// point, which the caller traps, as there is no f8.
fn widen_from_byte(op: FpWidenConvertOp, raw: u64, frm: RoundingMode) -> Option<(u64, FpFlags)> {
    let value = match op {
        FpWidenConvertOp::Int(IntConvert::FromInt(Signedness::Unsigned)) => f64::from(raw as u8),
        FpWidenConvertOp::Int(IntConvert::FromInt(Signedness::Signed)) => {
            Sew::E8.sign_extend(raw) as f64
        }
        FpWidenConvertOp::Int(IntConvert::ToInt(..)) | FpWidenConvertOp::Float => return None,
    };
    let (bits, flags) = f64_to_f16(value, frm);
    Some((u64::from(bits), flags))
}

/// NV when a widening operation has a signaling NaN operand, whose host
/// conversion to the wider format raises NV before the flags are cleared.
const fn invalid_if(signaling_nan_operand: bool) -> FpFlags {
    if signaling_nan_operand { FpFlags::NV } else { FpFlags::NONE }
}

/// Widening FMA operations, accumulating into `vd` at 2*SEW.
pub(super) fn exec_fp_widening_fma(
    signs: FmaSigns,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    operand1: VecOperand,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let element: fn(FmaSigns, [u64; 3], RoundingMode) -> (u64, FpFlags) = match ctx.sew {
        Sew::E32 => widening_fma::<f32, f64>,
        Sew::E16 => widening_fma::<SoftF16, f32>,
        Sew::E8 | Sew::E64 => return VecExecResult::default(),
    };
    let Some(wsew) = widen_sew(ctx.sew) else {
        return VecExecResult::default();
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    for i in 0..vlmax {
        if ctx.skip_element(vpr, i, FillTarget::Element(vd_idx, wsew)) {
            continue;
        }
        let vs2 = vpr.read_element(vs2_idx, ElemIdx::new(i), ctx.sew);
        let op1 = read_op1(vpr, &operand1, i, ctx.sew);
        let vd = vpr.read_element(vd_idx, ElemIdx::new(i), wsew);
        let (result, f) = element(signs, [vs2, op1, vd], ctx.frm);
        flags = flags | f;
        vpr.write_element(vd_idx, ElemIdx::new(i), wsew, result);
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// One element of a widening FMA: `vs2` and `op1` at `N`, the `vd`
/// accumulator at `W`, fused at `W`.
fn widening_fma<N: Float, W: Float + Neg<Output = W>>(
    signs: FmaSigns,
    [vs2, op1, vd]: [u64; 3],
    rm: RoundingMode,
) -> (u64, FpFlags) {
    let (vs2, vs2_signaling) = widened::<N, W>(vs2);
    let (op1, op1_signaling) = widened::<N, W>(op1);
    let vd = W::from_raw_bits(vd);
    let (a, b, c) = fma_operands((signs, FmaAddend::Vd), op1, vs2, vd);
    let (result, flags) = a.mul_add(b, c, rm);
    let signaling = vs2_signaling || op1_signaling || vd.is_signaling_nan();
    (result.canonical().raw_bits(), flags | invalid_if(signaling))
}

/// Narrowing FP conversions: read `vs2` at 2*SEW, write at SEW.
pub(super) fn exec_fp_narrowing(
    op: FpNarrowConvertOp,
    vpr: &mut impl VectorRegFile,
    vd_idx: VRegIdx,
    vs2_idx: VRegIdx,
    ctx: &VecExecCtx,
) -> VecExecResult {
    let Some(src_sew) = widen_sew(ctx.sew) else {
        return VecExecResult::default();
    };
    let vlmax = Vlmax::compute(vpr.vlen(), ctx.sew, ctx.vlmul).as_usize();
    let mut flags = FpFlags::NONE;

    for i in 0..vlmax {
        if ctx.skip_element(vpr, i, FillTarget::Element(vd_idx, ctx.sew)) {
            continue;
        }

        let vs2_raw = vpr.read_element(vs2_idx, ElemIdx::new(i), src_sew);
        let narrowed = match ctx.sew {
            Sew::E32 => Some(narrow_to_single(op, vs2_raw, ctx.frm)),
            Sew::E16 => Some(narrow_to_half(op, vs2_raw, ctx.frm)),
            Sew::E8 => narrow_to_byte(op, vs2_raw, ctx.frm),
            Sew::E64 => None,
        };
        if let Some((bits, f)) = narrowed {
            flags = flags | f;
            vpr.write_element(vd_idx, ElemIdx::new(i), ctx.sew, bits);
        }
    }

    VecExecResult { vxsat: false, scalar_result: None, fp_flags: flags }
}

/// A narrowing conversion of the 64-bit element `raw` to 32 bits.
fn narrow_to_single(op: FpNarrowConvertOp, raw: u64, frm: RoundingMode) -> (u64, FpFlags) {
    let value = elem_to_f64(raw);
    match op {
        FpNarrowConvertOp::Float => {
            let (r, f) = on_host_f32(frm, || black_box(value) as f32, || Exact::of_f64(value));
            (zext_word(box_f32_canon(r)), f)
        }
        // Round-to-odd: truncate to f32 precision and jam the LSB to 1 if
        // any bit was lost. It does not raise the inexact flag (per Zvfh).
        FpNarrowConvertOp::FloatRoundToOdd => {
            (zext_word(box_f32_canon(f64_to_f32_round_to_odd(value))), FpFlags::NONE)
        }
        FpNarrowConvertOp::Int(IntConvert::ToInt(signedness, rounding)) => {
            float_to_int(value, (signedness, rounding), 32, frm)
        }
        // The source is the full 64-bit integer; it must not be truncated first.
        FpNarrowConvertOp::Int(IntConvert::FromInt(Signedness::Unsigned)) => {
            let (r, f) =
                on_host_f32(frm, || black_box(raw) as f32, || Exact::of_unsigned(u128::from(raw)));
            (u64::from(r.to_bits()), f)
        }
        FpNarrowConvertOp::Int(IntConvert::FromInt(Signedness::Signed)) => {
            let signed = Sew::E64.sign_extend(raw);
            let (r, f) = on_host_f32(
                frm,
                || black_box(signed) as f32,
                || Exact::of_integer(i128::from(signed)),
            );
            (u64::from(r.to_bits()), f)
        }
    }
}

/// A narrowing conversion of the 32-bit element `raw` to 16 bits. f16
/// results need Zvfh, which the caller checked; 16-bit integers do not.
fn narrow_to_half(op: FpNarrowConvertOp, raw: u64, frm: RoundingMode) -> (u64, FpFlags) {
    let value = elem_to_f32(raw);
    match op {
        FpNarrowConvertOp::Float => {
            let (bits, f) = f64_to_f16(f64::from(value), frm);
            (u64::from(bits), f)
        }
        // Round-to-odd: round toward zero into f16, then jam the mantissa LSB
        // to 1 if rounding was inexact. It suppresses NX (per Zvfh).
        FpNarrowConvertOp::FloatRoundToOdd => {
            let (bits, f) = f64_to_f16(f64::from(value), RoundingMode::Rtz);
            let inexact = f.bits() & FpFlags::NX.bits() != 0;
            let finite = !value.is_nan() && !value.is_infinite();
            let jammed = if inexact && finite { bits | 1 } else { bits };
            (u64::from(jammed), FpFlags::from_bits(f.bits() & !FpFlags::NX.bits()))
        }
        FpNarrowConvertOp::Int(IntConvert::ToInt(signedness, rounding)) => {
            float_to_int(f64::from(value), (signedness, rounding), 16, frm)
        }
        FpNarrowConvertOp::Int(IntConvert::FromInt(Signedness::Unsigned)) => {
            let (bits, f) = f64_to_f16(f64::from(raw as u32), frm);
            (u64::from(bits), f)
        }
        FpNarrowConvertOp::Int(IntConvert::FromInt(Signedness::Signed)) => {
            let (bits, f) = f64_to_f16(f64::from(Sew::E32.sign_extend(raw) as i32), frm);
            (u64::from(bits), f)
        }
    }
}

/// A narrowing conversion of the f16 element `raw` to an 8-bit integer
/// (Zvfh); `None` for the conversions to floating point, which the caller
/// traps, as there is no f8.
fn narrow_to_byte(op: FpNarrowConvertOp, raw: u64, frm: RoundingMode) -> Option<(u64, FpFlags)> {
    let value = f64::from(f16_to_f32(raw as u16));
    match op {
        FpNarrowConvertOp::Int(IntConvert::ToInt(signedness, rounding)) => {
            Some(float_to_int(value, (signedness, rounding), 8, frm))
        }
        FpNarrowConvertOp::Int(IntConvert::FromInt(_))
        | FpNarrowConvertOp::Float
        | FpNarrowConvertOp::FloatRoundToOdd => None,
    }
}
