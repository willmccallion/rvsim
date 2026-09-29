//! Float-to-integer conversions and rounding under an explicit rounding mode.

use crate::isa::fp::{FpFlags, RoundingMode};

/// Compute one standard FP element at SEW=32.
///
/// Returns `(result_bits, flags)`.
#[allow(clippy::too_many_lines)]
/// Round `a` to an integer-valued float per FRM. Used by vfcvt.x.f / vfcvt.xu.f
/// because Rust's `f32 as i32` cast is hardcoded to round-toward-zero and does
/// not honour the host FPU's rounding mode.
#[inline]
pub(super) const fn round_f32_to_int_per_frm(a: f32, frm: RoundingMode) -> f32 {
    match frm {
        RoundingMode::Rne => a.round_ties_even(),
        RoundingMode::Rtz => a.trunc(),
        RoundingMode::Rdn => a.floor(),
        RoundingMode::Rup => a.ceil(),
        RoundingMode::Rmm => a.round(), // round half away from zero
    }
}

#[inline]
pub(super) const fn round_f64_to_int_per_frm(a: f64, frm: RoundingMode) -> f64 {
    match frm {
        RoundingMode::Rne => a.round_ties_even(),
        RoundingMode::Rtz => a.trunc(),
        RoundingMode::Rdn => a.floor(),
        RoundingMode::Rup => a.ceil(),
        RoundingMode::Rmm => a.round(),
    }
}

/// FRM-aware f64 → i16 conversion with saturation. Used by f16 vfcvt.x.f
/// (the f16 element is upcast to f64 by the caller before rounding).
pub(super) fn f64_to_i16_frm(a: f64, frm: RoundingMode) -> (i16, FpFlags) {
    if a.is_nan() {
        return (i16::MAX, FpFlags::NV);
    }
    let rounded = round_f64_to_int_per_frm(a, frm);
    if rounded < i16::MIN as f64 {
        return (i16::MIN, FpFlags::NV);
    }
    if rounded > i16::MAX as f64 {
        return (i16::MAX, FpFlags::NV);
    }
    let nx = if rounded == a { FpFlags::NONE } else { FpFlags::NX };
    (rounded as i16, nx)
}

/// FRM-aware f64 → u16 conversion with saturation.
pub(super) fn f64_to_u16_frm(a: f64, frm: RoundingMode) -> (u16, FpFlags) {
    if a.is_nan() {
        return (u16::MAX, FpFlags::NV);
    }
    let rounded = round_f64_to_int_per_frm(a, frm);
    if rounded < 0.0 {
        return (0, FpFlags::NV);
    }
    if rounded > u16::MAX as f64 {
        return (u16::MAX, FpFlags::NV);
    }
    let nx = if rounded == a { FpFlags::NONE } else { FpFlags::NX };
    (rounded as u16, nx)
}

/// FRM-aware f32 → i32 conversion with saturation and IEEE flag computation.
pub(super) fn f32_to_i32_frm(a: f32, frm: RoundingMode) -> (i32, FpFlags) {
    if a.is_nan() {
        return (i32::MAX, FpFlags::NV);
    }
    let rounded = round_f32_to_int_per_frm(a, frm);
    // Out-of-range detection: i32::MIN is exactly representable in f32, but
    // i32::MAX is not (rounds up to 2^31). Compare against the exact 2^31
    // boundary using f64 to avoid the rounding pitfall.
    let r64 = rounded as f64;
    if r64 < i32::MIN as f64 {
        return (i32::MIN, FpFlags::NV);
    }
    if r64 >= 2_147_483_648.0_f64 {
        return (i32::MAX, FpFlags::NV);
    }
    let result = rounded as i32;
    let nx = if rounded == a { FpFlags::NONE } else { FpFlags::NX };
    (result, nx)
}

/// FRM-aware f32 → u32 conversion with saturation.
pub(super) fn f32_to_u32_frm(a: f32, frm: RoundingMode) -> (u32, FpFlags) {
    if a.is_nan() {
        return (u32::MAX, FpFlags::NV);
    }
    let rounded = round_f32_to_int_per_frm(a, frm);
    let r64 = rounded as f64;
    if r64 < 0.0 {
        return (0, FpFlags::NV);
    }
    if r64 >= 4_294_967_296.0_f64 {
        return (u32::MAX, FpFlags::NV);
    }
    let result = rounded as u32;
    let nx = if rounded == a { FpFlags::NONE } else { FpFlags::NX };
    (result, nx)
}

/// FRM-aware f64 → i32 conversion with saturation (used by narrowing cvt).
pub(super) fn f64_to_i32_frm(a: f64, frm: RoundingMode) -> (i32, FpFlags) {
    if a.is_nan() {
        return (i32::MAX, FpFlags::NV);
    }
    let rounded = round_f64_to_int_per_frm(a, frm);
    if rounded < i32::MIN as f64 {
        return (i32::MIN, FpFlags::NV);
    }
    if rounded >= 2_147_483_648.0_f64 {
        return (i32::MAX, FpFlags::NV);
    }
    let result = rounded as i32;
    let nx = if rounded == a { FpFlags::NONE } else { FpFlags::NX };
    (result, nx)
}

/// FRM-aware f64 → u32 conversion with saturation (used by narrowing cvt).
pub(super) fn f64_to_u32_frm(a: f64, frm: RoundingMode) -> (u32, FpFlags) {
    if a.is_nan() {
        return (u32::MAX, FpFlags::NV);
    }
    let rounded = round_f64_to_int_per_frm(a, frm);
    if rounded < 0.0 {
        return (0, FpFlags::NV);
    }
    if rounded >= 4_294_967_296.0_f64 {
        return (u32::MAX, FpFlags::NV);
    }
    let result = rounded as u32;
    let nx = if rounded == a { FpFlags::NONE } else { FpFlags::NX };
    (result, nx)
}

/// FRM-aware f64 → i64 conversion with saturation.
pub(super) fn f64_to_i64_frm(a: f64, frm: RoundingMode) -> (i64, FpFlags) {
    if a.is_nan() {
        return (i64::MAX, FpFlags::NV);
    }
    let rounded = round_f64_to_int_per_frm(a, frm);
    if rounded < i64::MIN as f64 {
        return (i64::MIN, FpFlags::NV);
    }
    // 2^63 is the smallest f64 value strictly above i64::MAX.
    if rounded >= 9_223_372_036_854_775_808.0_f64 {
        return (i64::MAX, FpFlags::NV);
    }
    let result = rounded as i64;
    let nx = if rounded == a { FpFlags::NONE } else { FpFlags::NX };
    (result, nx)
}

/// Round-to-odd narrowing of an f64 to an f32. Used by vfncvt.rod.f.f.w.
///
/// The semantic per RVV: "round to nearest, but if the result is inexact,
/// produce the value with the odd LSB". Equivalently, **truncate** to f32
/// precision and OR-jam the LSB to 1 if any bit was lost. This is *not*
/// the same as Rust's RNE-then-jam: RNE can round UP across the mantissa
/// boundary (e.g. 0x3FFFFFFFFFFFFFFF rounds to 2.0, not 1.999…),
/// producing a value with the wrong LSB after jamming.
pub(super) fn f64_to_f32_round_to_odd(a: f64) -> f32 {
    if a.is_nan() {
        return f32::NAN;
    }
    if a.is_infinite() {
        return if a.is_sign_positive() { f32::INFINITY } else { f32::NEG_INFINITY };
    }

    // Round-to-odd: convert with round-toward-zero, then jam the LSB to 1
    // if the conversion was inexact (so the result is always odd when
    // narrowing loses precision). For overflow, RTZ returns ±max-normal,
    // which already has LSB=1, so the jam is a no-op there.
    let abs = a.abs();
    let f32_max = f32::from_bits(0x7F7F_FFFF);

    let truncated_f32 = if abs > f32_max as f64 {
        // Overflow: RTZ saturates to ±max-normal.
        if a.is_sign_positive() { f32_max } else { -f32_max }
    } else {
        // Mask off bits beyond f32 precision; the truncated value is exact
        // in f32 (≤ 23 mantissa bits set after masking).
        const LOST_MASK: u64 = (1u64 << 29) - 1;
        let truncated_bits = a.to_bits() & !LOST_MASK;
        f64::from_bits(truncated_bits) as f32
    };

    let lost = a.to_bits() & ((1u64 << 29) - 1) != 0 || abs > f32_max as f64;
    if lost && truncated_f32.is_finite() {
        f32::from_bits(truncated_f32.to_bits() | 1)
    } else {
        truncated_f32
    }
}

/// FRM-aware f64 → u64 conversion with saturation.
pub(super) fn f64_to_u64_frm(a: f64, frm: RoundingMode) -> (u64, FpFlags) {
    if a.is_nan() {
        return (u64::MAX, FpFlags::NV);
    }
    let rounded = round_f64_to_int_per_frm(a, frm);
    if rounded < 0.0 {
        return (0, FpFlags::NV);
    }
    if rounded >= 18_446_744_073_709_551_616.0_f64 {
        return (u64::MAX, FpFlags::NV);
    }
    let result = rounded as u64;
    let nx = if rounded == a { FpFlags::NONE } else { FpFlags::NX };
    (result, nx)
}

/// FRM-aware f32 → i64 conversion (used by widening vfwcvt). Promotes to
/// f64 first (lossless for f32) and reuses `f64_to_i64_frm`.
pub(super) fn f32_to_i64_frm(a: f32, frm: RoundingMode) -> (i64, FpFlags) {
    if a.is_nan() {
        return (i64::MAX, FpFlags::NV);
    }
    f64_to_i64_frm(a as f64, frm)
}

/// FRM-aware f32 → u64 conversion (used by widening vfwcvt).
pub(super) fn f32_to_u64_frm(a: f32, frm: RoundingMode) -> (u64, FpFlags) {
    if a.is_nan() {
        return (u64::MAX, FpFlags::NV);
    }
    f64_to_u64_frm(a as f64, frm)
}
