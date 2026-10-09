//! Single-width floating-point arithmetic, element by element.

use super::convert::{
    f32_to_i32_frm, f32_to_u32_frm, f64_to_i16_frm, f64_to_i64_frm, f64_to_u16_frm, f64_to_u64_frm,
};
use super::estimate::{vfrec7_16, vfrec7_32, vfrec7_64, vfrsqrt7_16, vfrsqrt7_32, vfrsqrt7_64};
use super::{F32_SIGN_BIT, F64_SIGN_BIT, elem_to_f32, elem_to_f64};
use crate::exec::compute::fpu::exact::{self, Exact, on_host_f32, on_host_f64};
use crate::exec::compute::fpu::half::{
    CANONICAL_NAN_F16, classify_f16, f16_to_f32, f64_to_f16, is_snan_f16, product_is_invalid,
    sum_is_invalid,
};
use crate::exec::compute::fpu::nan_handling::{
    box_f32_canon, canonicalize_f64_bits, fmax_f32, fmax_f64, fmin_f32, fmin_f64,
};
use crate::exec::compute::fpu::nan_handling::{is_snan_f32, is_snan_f64};
use crate::exec::compute::vector::context::{
    VecExecCtx, VecExecResult, VecOperand, mask_active, read_op1, sign_extend,
};
use crate::exec::compute::vector::regfile::VectorRegFile;
use crate::isa::fp::{FpFlags, RoundingMode};
use crate::isa::op::VectorOp;
use crate::isa::rvv::{ElemIdx, Sew, VRegIdx, Vlmax};
use std::hint::black_box;

/// RISC-V FCLASS for f32: returns 10-bit classification bitmask.
pub(super) const fn classify_f32(val: u32) -> u64 {
    let sign = (val >> 31) & 1;
    let exp = (val >> 23) & 0xFF;
    let frac = val & 0x007F_FFFF;

    if exp == 0xFF && frac != 0 {
        // NaN
        if frac & 0x0040_0000 != 0 {
            1 << 9 // qNaN
        } else {
            1 << 8 // sNaN
        }
    } else if exp == 0xFF {
        if sign != 0 { 1 << 0 } else { 1 << 7 } // ±inf
    } else if exp == 0 && frac == 0 {
        if sign != 0 { 1 << 3 } else { 1 << 4 } // ±zero
    } else if exp == 0 {
        if sign != 0 { 1 << 2 } else { 1 << 5 } // ±subnormal
    } else if sign != 0 {
        1 << 1 // negative normal
    } else {
        1 << 6 // positive normal
    }
}

/// RISC-V FCLASS for f64: returns 10-bit classification bitmask.
pub(super) const fn classify_f64(val: u64) -> u64 {
    let sign = (val >> 63) & 1;
    let exp = (val >> 52) & 0x7FF;
    let frac = val & 0x000F_FFFF_FFFF_FFFF;

    if exp == 0x7FF && frac != 0 {
        if frac & 0x0008_0000_0000_0000 != 0 {
            1 << 9 // qNaN
        } else {
            1 << 8 // sNaN
        }
    } else if exp == 0x7FF {
        if sign != 0 { 1 << 0 } else { 1 << 7 } // ±inf
    } else if exp == 0 && frac == 0 {
        if sign != 0 { 1 << 3 } else { 1 << 4 } // ±zero
    } else if exp == 0 {
        if sign != 0 { 1 << 2 } else { 1 << 5 } // ±subnormal
    } else if sign != 0 {
        1 << 1
    } else {
        1 << 6
    }
}

pub(super) fn compute_f32(
    op: VectorOp,
    vs2_bits: u64,
    op1_bits: u64,
    frm: RoundingMode,
) -> (u64, FpFlags) {
    let a = elem_to_f32(vs2_bits);
    let b = elem_to_f32(op1_bits);

    match op {
        VectorOp::VFAdd => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(a) + black_box(b),
                || exact::add(Exact::of_f32(a), Exact::of_f32(b)),
            );
            (box_f32_canon(r), flags)
        }
        VectorOp::VFSub => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(a) - black_box(b),
                || exact::sub(Exact::of_f32(a), Exact::of_f32(b)),
            );
            (box_f32_canon(r), flags)
        }
        VectorOp::VFRSub => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(b) - black_box(a),
                || exact::sub(Exact::of_f32(b), Exact::of_f32(a)),
            );
            (box_f32_canon(r), flags)
        }
        VectorOp::VFMul => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(a) * black_box(b),
                || exact::mul(Exact::of_f32(a), Exact::of_f32(b)),
            );
            (box_f32_canon(r), flags)
        }
        VectorOp::VFDiv => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(a) / black_box(b),
                || exact::div(Exact::of_f32(a), Exact::of_f32(b)),
            );
            (box_f32_canon(r), flags)
        }
        VectorOp::VFRDiv => {
            let (r, flags) = on_host_f32(
                frm,
                || black_box(b) / black_box(a),
                || exact::div(Exact::of_f32(b), Exact::of_f32(a)),
            );
            (box_f32_canon(r), flags)
        }
        VectorOp::VFSqrt => {
            let (r, flags) =
                on_host_f32(frm, || black_box(a).sqrt(), || exact::sqrt(Exact::of_f32(a)));
            (box_f32_canon(r), flags)
        }
        VectorOp::VFRsqrt7 => vfrsqrt7_32(vs2_bits as u32),
        VectorOp::VFRec7 => vfrec7_32(vs2_bits as u32, frm),
        VectorOp::VFMin => {
            let r = fmin_f32(a, b);
            // IEEE 754-2008 minNum: raise NV if either operand is a signaling NaN.
            let f = if is_snan_f32(a) || is_snan_f32(b) { FpFlags::NV } else { FpFlags::NONE };
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, f)
        }
        VectorOp::VFMax => {
            let r = fmax_f32(a, b);
            let f = if is_snan_f32(a) || is_snan_f32(b) { FpFlags::NV } else { FpFlags::NONE };
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, f)
        }
        VectorOp::VFSgnj => {
            let r = f32::from_bits((a.to_bits() & !F32_SIGN_BIT) | (b.to_bits() & F32_SIGN_BIT));
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
        }
        VectorOp::VFSgnjn => {
            let r = f32::from_bits((a.to_bits() & !F32_SIGN_BIT) | (!b.to_bits() & F32_SIGN_BIT));
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
        }
        VectorOp::VFSgnjx => {
            let r = f32::from_bits(a.to_bits() ^ (b.to_bits() & F32_SIGN_BIT));
            (r.to_bits() as u64 | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
        }
        VectorOp::VFClass => (classify_f32(vs2_bits as u32), FpFlags::NONE),
        // Conversions: float -> unsigned int (uses dynamic FRM)
        VectorOp::VFCvtXuF => {
            let (r, f) = f32_to_u32_frm(a, frm);
            (r as u64, f)
        }
        // Conversions: float -> signed int (uses dynamic FRM)
        VectorOp::VFCvtXF => {
            let (r, f) = f32_to_i32_frm(a, frm);
            (r as u32 as u64, f)
        }
        // Conversions with explicit RTZ: float -> unsigned int
        VectorOp::VFCvtRtzXuF => {
            let (r, f) = f32_to_u32_frm(a, RoundingMode::Rtz);
            (r as u64, f)
        }
        // Conversions with explicit RTZ: float -> signed int
        VectorOp::VFCvtRtzXF => {
            let (r, f) = f32_to_i32_frm(a, RoundingMode::Rtz);
            (r as u32 as u64, f)
        }
        // Conversions: unsigned int -> float
        VectorOp::VFCvtFXu => {
            let unsigned = vs2_bits as u32;
            let (r, flags) = on_host_f32(
                frm,
                || black_box(unsigned) as f32,
                || Exact::of_unsigned(u128::from(unsigned)),
            );
            (box_f32_canon(r), flags)
        }
        // Conversions: signed int -> float
        VectorOp::VFCvtFX => {
            let signed = sign_extend(vs2_bits, Sew::E32) as i32;
            let (r, flags) = on_host_f32(
                frm,
                || black_box(signed) as f32,
                || Exact::of_integer(i128::from(signed)),
            );
            (box_f32_canon(r), flags)
        }
        _ => (0, FpFlags::NONE),
    }
}

/// Compute one standard FP element at SEW=64.
///
/// Returns `(result_bits, flags)`.
#[allow(clippy::too_many_lines)]
pub(super) fn compute_f64(
    op: VectorOp,
    vs2_bits: u64,
    op1_bits: u64,
    frm: RoundingMode,
) -> (u64, FpFlags) {
    let a = elem_to_f64(vs2_bits);
    let b = elem_to_f64(op1_bits);

    match op {
        VectorOp::VFAdd => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(a) + black_box(b),
                || exact::add(Exact::of_f64(a), Exact::of_f64(b)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        VectorOp::VFSub => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(a) - black_box(b),
                || exact::sub(Exact::of_f64(a), Exact::of_f64(b)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        VectorOp::VFRSub => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(b) - black_box(a),
                || exact::sub(Exact::of_f64(b), Exact::of_f64(a)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        VectorOp::VFMul => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(a) * black_box(b),
                || exact::mul(Exact::of_f64(a), Exact::of_f64(b)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        VectorOp::VFDiv => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(a) / black_box(b),
                || exact::div(Exact::of_f64(a), Exact::of_f64(b)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        VectorOp::VFRDiv => {
            let (r, flags) = on_host_f64(
                frm,
                || black_box(b) / black_box(a),
                || exact::div(Exact::of_f64(b), Exact::of_f64(a)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        VectorOp::VFSqrt => {
            let (r, flags) =
                on_host_f64(frm, || black_box(a).sqrt(), || exact::sqrt(Exact::of_f64(a)));
            (canonicalize_f64_bits(r), flags)
        }
        VectorOp::VFRsqrt7 => vfrsqrt7_64(vs2_bits),
        VectorOp::VFRec7 => vfrec7_64(vs2_bits, frm),
        VectorOp::VFMin => {
            let r = fmin_f64(a, b);
            let f = if is_snan_f64(a) || is_snan_f64(b) { FpFlags::NV } else { FpFlags::NONE };
            (r.to_bits(), f)
        }
        VectorOp::VFMax => {
            let r = fmax_f64(a, b);
            let f = if is_snan_f64(a) || is_snan_f64(b) { FpFlags::NV } else { FpFlags::NONE };
            (r.to_bits(), f)
        }
        VectorOp::VFSgnj => {
            let r = f64::from_bits((a.to_bits() & !F64_SIGN_BIT) | (b.to_bits() & F64_SIGN_BIT));
            (r.to_bits(), FpFlags::NONE)
        }
        VectorOp::VFSgnjn => {
            let r = f64::from_bits((a.to_bits() & !F64_SIGN_BIT) | (!b.to_bits() & F64_SIGN_BIT));
            (r.to_bits(), FpFlags::NONE)
        }
        VectorOp::VFSgnjx => {
            let r = f64::from_bits(a.to_bits() ^ (b.to_bits() & F64_SIGN_BIT));
            (r.to_bits(), FpFlags::NONE)
        }
        VectorOp::VFClass => (classify_f64(vs2_bits), FpFlags::NONE),
        VectorOp::VFCvtXuF => {
            let (r, f) = f64_to_u64_frm(a, frm);
            (r, f)
        }
        VectorOp::VFCvtXF => {
            let (r, f) = f64_to_i64_frm(a, frm);
            (r as u64, f)
        }
        VectorOp::VFCvtRtzXuF => {
            let (r, f) = f64_to_u64_frm(a, RoundingMode::Rtz);
            (r, f)
        }
        VectorOp::VFCvtRtzXF => {
            let (r, f) = f64_to_i64_frm(a, RoundingMode::Rtz);
            (r as u64, f)
        }
        VectorOp::VFCvtFXu => {
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
        VectorOp::VFCvtFX => {
            let signed = vs2_bits as i64;
            let (r, flags) = on_host_f64(
                frm,
                || black_box(signed) as f64,
                || Exact::of_integer(i128::from(signed)),
            );
            (canonicalize_f64_bits(r), flags)
        }
        _ => (0, FpFlags::NONE),
    }
}

/// Compute one standard FP element at SEW=16 (Zvfh).
///
/// Vector f16 values are stored as raw u16 bit patterns in the low 16 bits of
/// each element slot — they are NOT NaN-boxed (that is only for scalar registers).
/// Arithmetic is performed by upcasting to f64 (lossless for f16 inputs) and
/// software-rounding back to f16 via `f64_to_f16`.
#[allow(clippy::too_many_lines)]
pub(super) fn compute_f16(
    op: VectorOp,
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
        VectorOp::VFAdd => round(fa + fb, invalid(sum_is_invalid(fa, fb))),
        VectorOp::VFSub => round(fa - fb, invalid(sum_is_invalid(fa, -fb))),
        VectorOp::VFRSub => round(fb - fa, invalid(sum_is_invalid(fb, -fa))),
        VectorOp::VFMul => round(fa * fb, invalid(product_is_invalid(fa, fb))),
        VectorOp::VFDiv => {
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
        VectorOp::VFRDiv => {
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
        VectorOp::VFSqrt => {
            let mut extra = FpFlags::NONE;
            if is_snan_f16(ha) || fa < 0.0 {
                extra = extra | FpFlags::NV;
            }
            round(fa.sqrt(), extra)
        }
        VectorOp::VFMin => {
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
        VectorOp::VFMax => {
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
        VectorOp::VFSgnj => ((ha & 0x7FFF) as u64 | (hb & 0x8000) as u64, FpFlags::NONE),
        VectorOp::VFSgnjn => ((ha & 0x7FFF) as u64 | (!hb & 0x8000) as u64, FpFlags::NONE),
        VectorOp::VFSgnjx => ((ha ^ (hb & 0x8000)) as u64, FpFlags::NONE),
        VectorOp::VFClass => (classify_f16(ha), FpFlags::NONE),
        VectorOp::VFCvtXuF => {
            let (r, f) = f64_to_u16_frm(fa, rm);
            (r as u64, f)
        }
        VectorOp::VFCvtXF => {
            let (r, f) = f64_to_i16_frm(fa, rm);
            (r as u16 as u64, f)
        }
        VectorOp::VFCvtRtzXuF => {
            let (r, f) = f64_to_u16_frm(fa, RoundingMode::Rtz);
            (r as u64, f)
        }
        VectorOp::VFCvtRtzXF => {
            let (r, f) = f64_to_i16_frm(fa, RoundingMode::Rtz);
            (r as u16 as u64, f)
        }
        VectorOp::VFCvtFXu => round(vs2_bits as u16 as f64, FpFlags::NONE),
        VectorOp::VFCvtFX => round(sign_extend(vs2_bits, Sew::E16) as i16 as f64, FpFlags::NONE),
        VectorOp::VFRsqrt7 => {
            let (r, f) = vfrsqrt7_16(ha);
            (r as u64, f)
        }
        VectorOp::VFRec7 => {
            let (r, f) = vfrec7_16(ha, rm);
            (r as u64, f)
        }
        _ => (0, FpFlags::NONE),
    }
}

/// Standard (non-widening, non-narrowing, non-FMA) FP element-wise loop.
pub(super) fn exec_fp_standard(
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
