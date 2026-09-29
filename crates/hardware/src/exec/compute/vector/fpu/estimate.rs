//! `vfrsqrt7` and `vfrec7`: the 7-bit reciprocal square-root and reciprocal
//! estimates, from the spec's lookup tables.

use crate::exec::compute::fpu::half::CANONICAL_NAN_F16;
use crate::exec::compute::fpu::nan_handling::{box_f32_canon, canonicalize_f64_bits};
use crate::isa::fp::{FpFlags, RoundingMode};

/// vfrsqrt7 lookup table per RVV 1.0 §13.9. Indexed by
/// `{exp[0], sig[MSB-1:MSB-6]}` (7 bits). `RSQRT7_TABLE[0..64]` →
/// exp[0]=0, `RSQRT7_TABLE[64..128]` → exp[0]=1. Values copied from
/// `SoftFloat`'s `fall_reciprocal.c` so they match spike's reference exactly.
#[rustfmt::skip]
pub(super) static RSQRT7_TABLE: [u8; 128] = [
    // exp[0] = 0
     52,  51,  50,  48,  47,  46,  44,  43,
     42,  41,  40,  39,  38,  36,  35,  34,
     33,  32,  31,  30,  30,  29,  28,  27,
     26,  25,  24,  23,  23,  22,  21,  20,
     19,  19,  18,  17,  16,  16,  15,  14,
     14,  13,  12,  12,  11,  10,  10,   9,
      9,   8,   7,   7,   6,   6,   5,   4,
      4,   3,   3,   2,   2,   1,   1,   0,
    // exp[0] = 1
    127, 125, 123, 121, 119, 118, 116, 114,
    113, 111, 109, 108, 106, 105, 103, 102,
    100,  99,  97,  96,  95,  93,  92,  91,
     90,  88,  87,  86,  85,  84,  83,  82,
     80,  79,  78,  77,  76,  75,  74,  73,
     72,  71,  70,  70,  69,  68,  67,  66,
     65,  64,  63,  63,  62,  61,  60,  59,
     59,  58,  57,  56,  56,  55,  54,  53,
];

/// vfrec7 lookup table per RVV 1.0 §13.10. Indexed by `sig[MSB-1:MSB-7]`
/// (7 bits → 128 entries). Values copied from `SoftFloat`'s
/// `fall_reciprocal.c` so they match spike's reference exactly.
#[rustfmt::skip]
pub(super) static REC7_TABLE: [u8; 128] = [
    127, 125, 123, 121, 119, 117, 116, 114,
    112, 110, 109, 107, 105, 104, 102, 100,
     99,  97,  96,  94,  93,  91,  90,  88,
     87,  85,  84,  83,  81,  80,  79,  77,
     76,  75,  74,  72,  71,  70,  69,  68,
     66,  65,  64,  63,  62,  61,  60,  59,
     58,  57,  56,  55,  54,  53,  52,  51,
     50,  49,  48,  47,  46,  45,  44,  43,
     42,  41,  40,  40,  39,  38,  37,  36,
     35,  35,  34,  33,  32,  31,  31,  30,
     29,  28,  28,  27,  26,  25,  25,  24,
     23,  23,  22,  21,  21,  20,  19,  19,
     18,  17,  17,  16,  15,  15,  14,  14,
     13,  12,  12,  11,  11,  10,   9,   9,
      8,   8,   7,   7,   6,   5,   5,   4,
      4,   3,   3,   2,   2,   1,   1,   0,
];

/// Compute vfrsqrt7 for an f32 value. Returns `(result_bits, flags)`.
///
/// Mirrors spike's `rsqrte7(_, e=8, s=23)` (no rounding-mode argument
/// because vfrsqrt7 cannot overflow). Subnormal inputs are normalized
/// with an extra left-shift, so downstream bit extraction matches.
pub(super) fn vfrsqrt7_32(bits: u32) -> (u64, FpFlags) {
    let sign = bits as u64 >> 31;
    let mut exp = ((bits >> 23) & 0xFF) as u64;
    let mut sig = (bits & 0x007F_FFFF) as u64;

    if exp == 0xFF && sig != 0 {
        let f = if sig & 0x0040_0000 == 0 { FpFlags::NV } else { FpFlags::NONE };
        return (box_f32_canon(f32::NAN) & 0xFFFF_FFFF, f);
    }
    if exp == 0xFF && sig == 0 {
        if sign != 0 {
            return (box_f32_canon(f32::NAN) & 0xFFFF_FFFF, FpFlags::NV);
        }
        return (0xFFFF_FFFF_0000_0000, FpFlags::NONE);
    }
    if exp == 0 && sig == 0 {
        let r = if sign != 0 { 0xFF80_0000u64 } else { 0x7F80_0000u64 };
        return (r | 0xFFFF_FFFF_0000_0000, FpFlags::DZ);
    }
    if sign != 0 {
        return (box_f32_canon(f32::NAN) & 0xFFFF_FFFF, FpFlags::NV);
    }

    let sub = exp == 0;
    if sub {
        while (sig & (1u64 << 22)) == 0 {
            sig <<= 1;
            exp = exp.wrapping_sub(1);
        }
        sig = (sig << 1) & 0x007F_FFFF;
    }

    let idx = (((exp & 1) << 6) | ((sig >> 17) & 0x3F)) as usize;
    let out_sig = (RSQRT7_TABLE[idx] as u64) << 16;
    let out_exp = u64::midpoint(3 * 127, !exp);

    let result = (sign << 31) | ((out_exp & 0xFF) << 23) | (out_sig & 0x007F_FFFF);
    (result | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
}

/// Compute vfrsqrt7 for an f64 value. Returns `(result_bits, flags)`.
///
/// Same shape as `vfrsqrt7_32` but with `e=11`, `s=52`.
pub(super) fn vfrsqrt7_64(bits: u64) -> (u64, FpFlags) {
    let sign = bits >> 63;
    let mut exp = (bits >> 52) & 0x7FF;
    let mut sig = bits & 0x000F_FFFF_FFFF_FFFF;

    if exp == 0x7FF && sig != 0 {
        let f = if sig & 0x0008_0000_0000_0000 == 0 { FpFlags::NV } else { FpFlags::NONE };
        return (canonicalize_f64_bits(f64::NAN), f);
    }
    if exp == 0x7FF && sig == 0 {
        if sign != 0 {
            return (canonicalize_f64_bits(f64::NAN), FpFlags::NV);
        }
        return (0, FpFlags::NONE);
    }
    if exp == 0 && sig == 0 {
        let r: u64 = if sign != 0 { 0xFFF0_0000_0000_0000 } else { 0x7FF0_0000_0000_0000 };
        return (r, FpFlags::DZ);
    }
    if sign != 0 {
        return (canonicalize_f64_bits(f64::NAN), FpFlags::NV);
    }

    let sub = exp == 0;
    if sub {
        while (sig & (1u64 << 51)) == 0 {
            sig <<= 1;
            exp = exp.wrapping_sub(1);
        }
        sig = (sig << 1) & 0x000F_FFFF_FFFF_FFFF;
    }

    let idx = (((exp & 1) << 6) | ((sig >> 46) & 0x3F)) as usize;
    let out_sig = (RSQRT7_TABLE[idx] as u64) << 45;
    let out_exp = u64::midpoint(3 * 1023, !exp);

    let result = (sign << 63) | ((out_exp & 0x7FF) << 52) | (out_sig & 0x000F_FFFF_FFFF_FFFF);
    (result, FpFlags::NONE)
}

/// Compute vfrsqrt7 for an f16 value. Returns `(f16_result_bits, flags)`.
pub(super) fn vfrsqrt7_16(bits: u16) -> (u16, FpFlags) {
    let sign = bits >> 15;
    let mut exp = ((bits >> 10) & 0x1F) as u32;
    let mut sig = (bits & 0x03FF) as u32;

    if exp == 0x1F && sig != 0 {
        let f = if sig & 0x0200 == 0 { FpFlags::NV } else { FpFlags::NONE };
        return (CANONICAL_NAN_F16, f);
    }
    if exp == 0x1F && sig == 0 {
        if sign != 0 {
            return (CANONICAL_NAN_F16, FpFlags::NV);
        }
        return (0, FpFlags::NONE);
    }
    if exp == 0 && sig == 0 {
        let r: u16 = if sign != 0 { 0xFC00 } else { 0x7C00 };
        return (r, FpFlags::DZ);
    }
    if sign != 0 {
        return (CANONICAL_NAN_F16, FpFlags::NV);
    }

    let sub = exp == 0;
    if sub {
        // Match spike's rsqrte7: normalize then shift left ONE more time so
        // the leading 1 falls off and the table-index extraction uses the
        // post-normalization mantissa positions.
        while (sig & 0x200) == 0 {
            sig <<= 1;
            exp = exp.wrapping_sub(1);
        }
        sig = (sig << 1) & 0x3FF;
    }

    // idx = {exp[0], sig[s-2:s-p-1]} with p=7, s=10 → top 6 fraction bits.
    let idx = (((exp & 1) << 6) | ((sig >> 4) & 0x3F)) as usize;
    let out_sig = RSQRT7_TABLE[idx] as u32;

    // out_exp = (3*bias - 1 - exp) / 2, computed in u64 to mirror spike's
    // u64 wrap-around arithmetic for negative `exp` values.
    let out_exp = u32::midpoint(3 * 15, !exp) & 0x1F;

    let result = ((sign as u32) << 15) | (out_exp << 10) | (out_sig << 3);
    (result as u16, FpFlags::NONE)
}

/// Compute vfrec7 for an f16 value. Returns `(f16_result_bits, flags)`.
///
/// Mirrors spike's `recip7` (`softfloat/fall_reciprocal.c)`: normalize a
/// subnormal input with an extra left-shift, saturate when normalization
/// pushes exp past −1, then look up sig[s-2:s-p-1] in the 7-bit table and
/// adjust `out_sig` for the two narrow subnormal-output cases (`out_exp` == 0
/// or `out_exp` == −1 in 64-bit unsigned).
pub(super) fn vfrec7_16(bits: u16, frm: RoundingMode) -> (u16, FpFlags) {
    let sign = (bits >> 15) as u32;
    let mut exp = ((bits >> 10) & 0x1F) as u64;
    let mut sig = (bits & 0x03FF) as u64;

    if exp == 0x1F && sig != 0 {
        let f = if sig & 0x200 == 0 { FpFlags::NV } else { FpFlags::NONE };
        return (CANONICAL_NAN_F16, f);
    }
    if exp == 0x1F && sig == 0 {
        return ((sign << 15) as u16, FpFlags::NONE);
    }
    if exp == 0 && sig == 0 {
        return (((sign << 15) | 0x7C00) as u16, FpFlags::DZ);
    }

    let sub = exp == 0;
    if sub {
        while (sig & 0x200) == 0 {
            sig <<= 1;
            exp = exp.wrapping_sub(1);
        }
        sig = (sig << 1) & 0x3FF;
        // If normalization pushed exp below -1 (i.e. exp != 0 and != UINT64_MAX),
        // the input was small enough that the reciprocal would overflow —
        // saturate per spike's rounding-mode-dependent return.
        if exp != 0 && exp != u64::MAX {
            let saturate_to_max_normal = matches!(
                frm,
                RoundingMode::Rtz
                    | RoundingMode::Rdn  // toward -inf: positive saturates down
                    | RoundingMode::Rup, // toward +inf: negative saturates up
            ) && match frm {
                RoundingMode::Rdn => sign == 0,
                RoundingMode::Rup => sign != 0,
                _ => true,
            };
            let abs_max = if saturate_to_max_normal { 0x7BFF } else { 0x7C00 };
            return (((sign << 15) | abs_max) as u16, FpFlags::OF | FpFlags::NX);
        }
    }

    let idx = ((sig >> 3) & 0x7F) as usize;
    let mut out_sig = (REC7_TABLE[idx] as u64) << 3;
    // out_exp uses spike's u64 wrap-around: `2*15 + ~exp`.
    let mut out_exp = 30u64.wrapping_add(!exp);

    // Narrow subnormal-output cases: shift out_sig down by 1 (or 2 when
    // out_exp wrapped to UINT64_MAX) and OR the implicit-1 into bit s-1.
    if out_exp == 0 || out_exp == u64::MAX {
        out_sig = (out_sig >> 1) | (1u64 << 9);
        if out_exp == u64::MAX {
            out_sig >>= 1;
            out_exp = 0;
        }
    }

    let result = ((sign as u64) << 15) | ((out_exp & 0x1F) << 10) | (out_sig & 0x3FF);
    (result as u16, FpFlags::NONE)
}

/// Saturation result for vfrec7 of a very small subnormal input. RNE/RMM
/// produce `±inf` (with OF + NX); RTZ and "round-toward-the-other-side"
/// rounding modes produce `±max-normal`. Mirrors spike's recip7 logic.
#[inline]
pub(super) const fn vfrec7_saturate_to_max(sign: u64, frm: RoundingMode) -> bool {
    match frm {
        RoundingMode::Rtz => true,
        RoundingMode::Rdn => sign == 0,
        RoundingMode::Rup => sign != 0,
        _ => false,
    }
}

/// Compute vfrec7 for an f32 value. Returns `(result_bits, flags)`.
///
/// Mirrors spike's `recip7(_, e=8, s=23)`: normalize subnormal input with
/// an extra left-shift, saturate when the normalized exp can't be
/// represented (exp ∉ {0, −1}), then look up sig[s-2:s-p-1] in the 7-bit
/// table and adjust `out_sig` for the two narrow subnormal-output cases.
pub(super) fn vfrec7_32(bits: u32, frm: RoundingMode) -> (u64, FpFlags) {
    let sign = bits as u64 >> 31;
    let mut exp = ((bits >> 23) & 0xFF) as u64;
    let mut sig = (bits & 0x007F_FFFF) as u64;

    if exp == 0xFF && sig != 0 {
        let f = if sig & 0x0040_0000 == 0 { FpFlags::NV } else { FpFlags::NONE };
        return (box_f32_canon(f32::NAN) & 0xFFFF_FFFF, f);
    }
    if exp == 0xFF && sig == 0 {
        return ((sign << 31) | 0xFFFF_FFFF_0000_0000, FpFlags::NONE);
    }
    if exp == 0 && sig == 0 {
        return (((sign << 31) | 0x7F80_0000) | 0xFFFF_FFFF_0000_0000, FpFlags::DZ);
    }

    let sub = exp == 0;
    if sub {
        while (sig & (1 << 22)) == 0 {
            sig <<= 1;
            exp = exp.wrapping_sub(1);
        }
        sig = (sig << 1) & 0x007F_FFFF;
        if exp != 0 && exp != u64::MAX {
            let to_max = vfrec7_saturate_to_max(sign, frm);
            let sat = if to_max { 0x7F7F_FFFFu64 } else { 0x7F80_0000u64 };
            return (((sign << 31) | sat) | 0xFFFF_FFFF_0000_0000, FpFlags::OF | FpFlags::NX);
        }
    }

    let idx = ((sig >> 16) & 0x7F) as usize;
    let mut out_sig = (REC7_TABLE[idx] as u64) << 16;
    let mut out_exp = 254u64.wrapping_add(!exp);

    if out_exp == 0 || out_exp == u64::MAX {
        out_sig = (out_sig >> 1) | (1u64 << 22);
        if out_exp == u64::MAX {
            out_sig >>= 1;
            out_exp = 0;
        }
    }

    let result = (sign << 31) | ((out_exp & 0xFF) << 23) | (out_sig & 0x007F_FFFF);
    (result | 0xFFFF_FFFF_0000_0000, FpFlags::NONE)
}

/// Compute vfrec7 for an f64 value. Returns `(result_bits, flags)`.
///
/// Same shape as `vfrec7_32` but with `e=11`, `s=52`.
pub(super) fn vfrec7_64(bits: u64, frm: RoundingMode) -> (u64, FpFlags) {
    let sign = bits >> 63;
    let mut exp = (bits >> 52) & 0x7FF;
    let mut sig = bits & 0x000F_FFFF_FFFF_FFFF;

    if exp == 0x7FF && sig != 0 {
        let f = if sig & 0x0008_0000_0000_0000 == 0 { FpFlags::NV } else { FpFlags::NONE };
        return (canonicalize_f64_bits(f64::NAN), f);
    }
    if exp == 0x7FF && sig == 0 {
        return (sign << 63, FpFlags::NONE);
    }
    if exp == 0 && sig == 0 {
        return ((sign << 63) | 0x7FF0_0000_0000_0000, FpFlags::DZ);
    }

    let sub = exp == 0;
    if sub {
        while (sig & (1u64 << 51)) == 0 {
            sig <<= 1;
            exp = exp.wrapping_sub(1);
        }
        sig = (sig << 1) & 0x000F_FFFF_FFFF_FFFF;
        if exp != 0 && exp != u64::MAX {
            let to_max = vfrec7_saturate_to_max(sign, frm);
            let sat: u64 = if to_max { 0x7FEF_FFFF_FFFF_FFFF } else { 0x7FF0_0000_0000_0000 };
            return ((sign << 63) | sat, FpFlags::OF | FpFlags::NX);
        }
    }

    let idx = ((sig >> 45) & 0x7F) as usize;
    let mut out_sig = (REC7_TABLE[idx] as u64) << 45;
    let mut out_exp = 2046u64.wrapping_add(!exp);

    if out_exp == 0 || out_exp == u64::MAX {
        out_sig = (out_sig >> 1) | (1u64 << 51);
        if out_exp == u64::MAX {
            out_sig >>= 1;
            out_exp = 0;
        }
    }

    let result = (sign << 63) | ((out_exp & 0x7FF) << 52) | (out_sig & 0x000F_FFFF_FFFF_FFFF);
    (result, FpFlags::NONE)
}
