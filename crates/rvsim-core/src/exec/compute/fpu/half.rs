//! Half-precision (Zfh) helpers: NaN-boxing, f16↔f32↔f64 conversions with
//! IEEE 754 rounding, classification, and signaling-NaN detection.
//!
//! The host has no native f16 type, so half-precision values are represented
//! as `u16` bit patterns. Arithmetic is performed by upcasting to `f64`
//! (lossless for add/sub/mul/fma of f16 inputs) and then software-rounding
//! back to f16 with the RISC-V rounding mode.

use crate::isa::fp::{FpClass, FpFlags, FpFormat, RoundingMode};

/// Canonical quiet NaN for IEEE 754 half-precision (sign=0, exp=all-1,
/// mantissa MSB=1, payload=0).
pub const CANONICAL_NAN_F16: u16 = 0x7E00;

/// Mask for validating an f16 NaN-box in a 64-bit register: upper 48 bits
/// must all be 1.
pub const F16_NAN_BOX_MASK: u64 = 0xFFFF_FFFF_FFFF_0000;

/// Boxes an f16 bit pattern into a 64-bit NaN-boxed register value.
#[inline]
pub const fn box_f16(bits: u16) -> u64 {
    (bits as u64) | F16_NAN_BOX_MASK
}

/// binary16's sign bit.
pub const F16_SIGN: u16 = 0x8000;

/// binary16 +∞: the exponent all ones and the fraction zero.
const F16_INFINITY: u16 = 0x7C00;

/// The largest finite binary16 magnitude.
const F16_MAX_FINITE: u16 = 0x7BFF;

/// Bits in binary16's fraction field.
const F16_FRAC_BITS: u32 = 10;

/// binary16's fraction field.
const F16_FRAC_MASK: u16 = (1 << F16_FRAC_BITS) - 1;

/// binary16's exponent field, shifted down: all ones for ±∞ and NaN.
const F16_EXP_ALL_ONES: u16 = 0x1F;

/// Bit position of binary16's sign.
const F16_SIGN_SHIFT: u32 = 15;

/// True iff the f16 bit pattern is a signaling NaN.
#[inline]
pub const fn is_snan_f16(bits: u16) -> bool {
    matches!(FpClass::of(FpFormat::Half, bits as u64), FpClass::SignalingNan)
}

/// Upconverts an IEEE 754 half-precision bit pattern to `f32` losslessly.
pub const fn f16_to_f32(h: u16) -> f32 {
    let sign = (h >> F16_SIGN_SHIFT) as u32;
    let exp = ((h >> F16_FRAC_BITS) & F16_EXP_ALL_ONES) as u32;
    let mant = (h & F16_FRAC_MASK) as u32;

    let bits: u32 = if exp == 0 {
        if mant == 0 {
            sign << 31
        } else {
            let mut m = mant;
            let mut e: i32 = -14;
            while (m & 0x400) == 0 {
                m <<= 1;
                e -= 1;
            }
            m &= F16_FRAC_MASK as u32;
            let new_exp = (e + 127) as u32;
            (sign << 31) | (new_exp << 23) | (m << 13)
        }
    } else if exp == F16_EXP_ALL_ONES as u32 {
        // Inf/NaN: align mantissa to f32 positions; quiet f16 NaN's MSB stays quiet.
        (sign << 31) | (0xFF << 23) | (mant << 13)
    } else {
        // Re-bias via i32: exp can be below 15 (would underflow u32).
        let new_exp = (exp as i32 - 15 + 127) as u32;
        (sign << 31) | (new_exp << 23) | (mant << 13)
    };

    f32::from_bits(bits)
}

/// Rounds a host `f64` value to an IEEE 754 half-precision bit pattern.
///
/// Returns `(bits, flags)` using the given RISC-V rounding mode. `flags`
/// carries the accrued `NX`/`UF`/`OF` flags; `NV` for NaN inputs is the
/// caller's responsibility.
pub fn f64_to_f16(val: f64, rm: RoundingMode) -> (u16, FpFlags) {
    let bits = val.to_bits();
    let sign_u16 = ((bits >> 63) & 1) as u16;
    let raw_exp = ((bits >> 52) & 0x7FF) as i32;
    let raw_mant = bits & 0x000F_FFFF_FFFF_FFFF;

    if raw_exp == 0x7FF && raw_mant != 0 {
        return (CANONICAL_NAN_F16, FpFlags::NONE);
    }
    if raw_exp == 0x7FF {
        return ((sign_u16 << F16_SIGN_SHIFT) | F16_INFINITY, FpFlags::NONE);
    }
    if raw_exp == 0 && raw_mant == 0 {
        return (sign_u16 << 15, FpFlags::NONE);
    }

    let (unbiased, sig): (i32, u64) = if raw_exp == 0 {
        let mut m = raw_mant;
        let mut e: i32 = -1022;
        while (m & 0x0010_0000_0000_0000) == 0 {
            m <<= 1;
            e -= 1;
        }
        (e, m)
    } else {
        (raw_exp - 1023, raw_mant | 0x0010_0000_0000_0000)
    };

    let mut flags = FpFlags::NONE;

    if unbiased >= 16 {
        flags = flags | FpFlags::OF | FpFlags::NX;
        return (overflow_result(sign_u16, rm), flags);
    }

    // 2^(-25) is half an f16 min subnormal; smaller rounds to 0 except
    // RDN of negatives / RUP of positives, which round to min subnormal.
    if unbiased < -25 {
        flags = flags | FpFlags::UF | FpFlags::NX;
        let result = match rm {
            RoundingMode::Rdn if sign_u16 == 1 => (sign_u16 << F16_SIGN_SHIFT) | 1,
            RoundingMode::Rup if sign_u16 == 0 => (sign_u16 << F16_SIGN_SHIFT) | 1,
            _ => sign_u16 << 15,
        };
        return (result, flags);
    }

    // Normal target: shift = 53 - 11 (incl. implicit bit). Subnormal: shift
    // more, losing leading bits. Lower bits become guard/round/sticky.
    let (f16_exp, shift): (u32, u32) = if unbiased >= -14 {
        ((unbiased + 15) as u32, 42)
    } else {
        (0, (42 + (-14 - unbiased)) as u32)
    };

    let mant_shifted = sig >> shift;
    let dropped = Discarded::of(sig, shift);

    let base_mant: u32 = if f16_exp > 0 {
        (mant_shifted as u32) & u32::from(F16_FRAC_MASK)
    } else {
        mant_shifted as u32
    };

    let inexact = dropped.is_inexact();
    let round_up = rounds_up(rm, sign_u16 == 1, dropped, base_mant & 1 == 1);

    let mut mant_out = base_mant;
    let mut exp_out = f16_exp;
    if round_up {
        mant_out += 1;
        if mant_out == 0x400 {
            mant_out = 0;
            exp_out = if f16_exp == 0 { 1 } else { exp_out + 1 };
        }
    }

    if inexact {
        flags = flags | FpFlags::NX;
    }
    // UF: inexact and tiny, judged after rounding to f16's precision with an
    // unbounded exponent (IEEE 754-2008 §7.5, as RISC-V does). Only a value
    // in [2^-15, 2^-14) can round out of the tiny range that way.
    let is_tiny = f16_exp == 0 && (unbiased < -15 || !reaches_smallest_normal(sig, sign_u16, rm));
    if is_tiny && inexact {
        flags = flags | FpFlags::UF;
    }

    if exp_out >= u32::from(F16_EXP_ALL_ONES) {
        flags = flags | FpFlags::OF | FpFlags::NX;
        return (overflow_result(sign_u16, rm), flags);
    }

    let result =
        (sign_u16 << F16_SIGN_SHIFT) | ((exp_out as u16) << F16_FRAC_BITS) | (mant_out as u16);
    (result, flags)
}

/// The bits rounding drops from a significand: the first, and whether any
/// later one is set.
#[derive(Clone, Copy)]
struct Discarded {
    round_bit: bool,
    sticky: bool,
}

impl Discarded {
    /// What dropping the low `bits` bits of `sig` discards.
    const fn of(sig: u64, bits: u32) -> Self {
        Self { round_bit: (sig >> (bits - 1)) & 1 == 1, sticky: sig & ((1 << (bits - 1)) - 1) != 0 }
    }

    /// Whether anything nonzero is discarded.
    const fn is_inexact(self) -> bool {
        self.round_bit || self.sticky
    }
}

/// Whether rounding by `rm` steps the kept significand, odd when `lsb_set`,
/// away from zero.
const fn rounds_up(rm: RoundingMode, is_negative: bool, dropped: Discarded, lsb_set: bool) -> bool {
    match rm {
        RoundingMode::Rne => dropped.round_bit && (dropped.sticky || lsb_set),
        RoundingMode::Rtz => false,
        RoundingMode::Rdn => is_negative && dropped.is_inexact(),
        RoundingMode::Rup => !is_negative && dropped.is_inexact(),
        RoundingMode::Rmm => dropped.round_bit,
    }
}

/// Whether the f64 significand `sig` of a value in [2^-15, 2^-14) rounds
/// by `rm` to 2^-14 at f16's full 11-bit precision.
const fn reaches_smallest_normal(sig: u64, sign_u16: u16, rm: RoundingMode) -> bool {
    const DROPPED_BITS: u32 = 42;
    const ALL_KEPT_BITS_SET: u64 = 0x7FF;
    let dropped = Discarded::of(sig, DROPPED_BITS);
    sig >> DROPPED_BITS == ALL_KEPT_BITS_SET && rounds_up(rm, sign_u16 == 1, dropped, true)
}

/// Produces the correct f16 result for an overflow under the given
/// rounding mode: either ±inf or the max finite magnitude.
#[inline]
const fn overflow_result(sign_u16: u16, rm: RoundingMode) -> u16 {
    let inf = (sign_u16 << F16_SIGN_SHIFT) | F16_INFINITY;
    let max_finite = (sign_u16 << F16_SIGN_SHIFT) | F16_MAX_FINITE;
    match rm {
        RoundingMode::Rtz => max_finite,
        RoundingMode::Rdn => {
            if sign_u16 == 0 {
                max_finite
            } else {
                inf
            }
        }
        RoundingMode::Rup => {
            if sign_u16 == 0 {
                inf
            } else {
                max_finite
            }
        }
        RoundingMode::Rne | RoundingMode::Rmm => inf,
    }
}

/// Whether `a + b` is an invalid operation: infinities of opposite sign.
#[must_use]
pub const fn sum_is_invalid(a: f64, b: f64) -> bool {
    a.is_infinite() && b.is_infinite() && a.is_sign_negative() != b.is_sign_negative()
}

/// Whether `a × b` is an invalid operation: zero times infinity.
#[must_use]
pub const fn product_is_invalid(a: f64, b: f64) -> bool {
    (a == 0.0 && b.is_infinite()) || (a.is_infinite() && b == 0.0)
}

/// Whether `a × b + c` is an invalid operation: the product is, even with a
/// quiet-NaN addend, or an infinite product meets an infinity of the other
/// sign.
#[must_use]
pub const fn fused_is_invalid(a: f64, b: f64, c: f64) -> bool {
    product_is_invalid(a, b) || (!a.is_nan() && !b.is_nan() && sum_is_invalid(a * b, c))
}
