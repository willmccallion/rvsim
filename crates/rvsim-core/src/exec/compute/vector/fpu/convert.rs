//! Round-to-odd narrowing of an f64 to an f32.

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

    let truncated_f32 = if abs > f64::from(f32_max) {
        // Overflow: RTZ saturates to ±max-normal.
        if a.is_sign_positive() { f32_max } else { -f32_max }
    } else {
        // Mask off bits beyond f32 precision; the truncated value is exact
        // in f32 (≤ 23 mantissa bits set after masking).
        const LOST_MASK: u64 = (1u64 << 29) - 1;
        let truncated_bits = a.to_bits() & !LOST_MASK;
        f64::from_bits(truncated_bits) as f32
    };

    let lost = a.to_bits() & ((1u64 << 29) - 1) != 0 || abs > f64::from(f32_max);
    if lost && truncated_f32.is_finite() {
        f32::from_bits(truncated_f32.to_bits() | 1)
    } else {
        truncated_f32
    }
}
