//! Half-precision (Zfh) helpers: NaN-boxing, f16↔f32↔f64 conversions with
//! IEEE 754 rounding, classification, and signaling-NaN detection.
//!
//! The host has no native f16 type, so half-precision values are represented
//! as `u16` bit patterns. Arithmetic is performed by upcasting to `f64`
//! (lossless for add/sub/mul/fma of f16 inputs) and then software-rounding
//! back to f16 with the RISC-V rounding mode.

use super::exception_flags::FpFlags;
use super::rounding_modes::RoundingMode;

/// Canonical quiet NaN for IEEE 754 half-precision (sign=0, exp=all-1,
/// mantissa MSB=1, payload=0).
pub const CANONICAL_NAN_F16: u16 = 0x7E00;

/// Mask for validating an f16 NaN-box in a 64-bit register: upper 48 bits
/// must all be 1.
pub const F16_NAN_BOX_MASK: u64 = 0xFFFF_FFFF_FFFF_0000;

/// Unboxes a 64-bit register value to an f16 bit pattern. Returns the
/// canonical quiet NaN if the value is not properly NaN-boxed.
#[inline]
pub const fn unbox_f16(val: u64) -> u16 {
    if (val & F16_NAN_BOX_MASK) == F16_NAN_BOX_MASK {
        val as u16
    } else {
        CANONICAL_NAN_F16
    }
}

/// Boxes an f16 bit pattern into a 64-bit NaN-boxed register value.
#[inline]
pub const fn box_f16(bits: u16) -> u64 {
    (bits as u64) | F16_NAN_BOX_MASK
}

/// True iff the f16 bit pattern is a signaling NaN.
#[inline]
pub const fn is_snan_f16(bits: u16) -> bool {
    let exp = (bits >> 10) & 0x1F;
    let mant = bits & 0x3FF;
    exp == 0x1F && mant != 0 && (mant & 0x200) == 0
}

/// RISC-V `FCLASS.H` result for a half-precision value: one-hot in
/// positions 0..9 identifying the value's class.
pub const fn classify_f16(bits: u16) -> u64 {
    let sign = (bits >> 15) & 1;
    let exp = (bits >> 10) & 0x1F;
    let mant = bits & 0x3FF;
    if exp == 0x1F && mant != 0 {
        if mant & 0x200 != 0 { 1 << 9 } else { 1 << 8 }
    } else if exp == 0x1F {
        if sign != 0 { 1 << 0 } else { 1 << 7 }
    } else if exp == 0 && mant == 0 {
        if sign != 0 { 1 << 3 } else { 1 << 4 }
    } else if exp == 0 {
        if sign != 0 { 1 << 2 } else { 1 << 5 }
    } else if sign != 0 {
        1 << 1
    } else {
        1 << 6
    }
}

/// Upconverts an IEEE 754 half-precision bit pattern to `f32` losslessly.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1F) as u32;
    let mant = (h & 0x3FF) as u32;

    let bits: u32 = if exp == 0 {
        if mant == 0 {
            sign << 31
        } else {
            // Subnormal: normalize the mantissa.
            let mut m = mant;
            let mut e: i32 = -14;
            while (m & 0x400) == 0 {
                m <<= 1;
                e -= 1;
            }
            m &= 0x3FF;
            let new_exp = (e + 127) as u32;
            (sign << 31) | (new_exp << 23) | (m << 13)
        }
    } else if exp == 0x1F {
        // Infinity or NaN: align mantissa to f32 positions. A quiet f16 NaN
        // has its MSB (bit 9) set, which becomes bit 22 in f32 — still quiet.
        (sign << 31) | (0xFF << 23) | (mant << 13)
    } else {
        let new_exp = exp - 15 + 127;
        (sign << 31) | (new_exp << 23) | (mant << 13)
    };

    f32::from_bits(bits)
}

/// Rounds a host `f64` value to an IEEE 754 half-precision bit pattern
/// using the given RISC-V rounding mode. Returns `(bits, flags)` where
/// `flags` carries the accrued `NX`/`UF`/`OF` flags (and `NV` for NaN
/// inputs is the caller's responsibility).
pub fn f64_to_f16(val: f64, rm: RoundingMode) -> (u16, FpFlags) {
    let bits = val.to_bits();
    let sign_u16 = ((bits >> 63) & 1) as u16;
    let raw_exp = ((bits >> 52) & 0x7FF) as i32;
    let raw_mant = bits & 0x000F_FFFF_FFFF_FFFF;

    // NaN: any NaN input becomes the canonical quiet f16 NaN. No flags.
    if raw_exp == 0x7FF && raw_mant != 0 {
        return (CANONICAL_NAN_F16, FpFlags::NONE);
    }
    // Infinity: preserve sign.
    if raw_exp == 0x7FF {
        return ((sign_u16 << 15) | 0x7C00, FpFlags::NONE);
    }
    // Zero: preserve sign.
    if raw_exp == 0 && raw_mant == 0 {
        return (sign_u16 << 15, FpFlags::NONE);
    }

    // Build the unbiased exponent and a significand with the leading 1 bit
    // explicit at position 52. For f64 subnormals we normalize it first.
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

    // Overflow: the value's magnitude is >= 2^16, which exceeds f16's
    // largest finite (2^16 - 2^5).
    if unbiased >= 16 {
        flags = flags | FpFlags::OF | FpFlags::NX;
        return (overflow_result(sign_u16, rm), flags);
    }

    // Below the smallest representable: 2^(-25) is half an f16 min subnormal
    // (f16 min subnormal = 2^(-24)). Anything smaller rounds to 0 except
    // for RDN of negatives / RUP of positives which round to min subnormal.
    if unbiased < -25 {
        flags = flags | FpFlags::UF | FpFlags::NX;
        let result = match rm {
            RoundingMode::Rdn if sign_u16 == 1 => (sign_u16 << 15) | 1,
            RoundingMode::Rup if sign_u16 == 0 => (sign_u16 << 15) | 1,
            _ => sign_u16 << 15,
        };
        return (result, flags);
    }

    // Shift the 53-bit significand down into f16's 10-bit mantissa field,
    // preserving guard/round/sticky below for rounding.
    // Normal f16 target: shift = 42 (= 53 - 11 = mantissa width difference
    // including the implicit bit).
    // Subnormal target: shift more, losing leading bits.
    let (f16_exp, shift): (u32, u32) = if unbiased >= -14 {
        ((unbiased + 15) as u32, 42)
    } else {
        (0, (42 + (-14 - unbiased)) as u32)
    };

    let mant_shifted = sig >> shift;
    let round_bit = (sig >> (shift - 1)) & 1;
    let sticky_mask = (1u64 << (shift - 1)) - 1;
    let sticky = (sig & sticky_mask) != 0;

    let base_mant: u32 = if f16_exp > 0 {
        (mant_shifted as u32) & 0x3FF
    } else {
        mant_shifted as u32
    };

    let inexact = round_bit != 0 || sticky;
    let round_up = match rm {
        RoundingMode::Rne => round_bit == 1 && (sticky || (base_mant & 1) == 1),
        RoundingMode::Rtz => false,
        RoundingMode::Rdn => sign_u16 == 1 && inexact,
        RoundingMode::Rup => sign_u16 == 0 && inexact,
        RoundingMode::Rmm => round_bit == 1,
    };

    let mut mant_out = base_mant;
    let mut exp_out = f16_exp;
    if round_up {
        mant_out += 1;
        if mant_out == 0x400 {
            // Mantissa rolled over into the exponent.
            mant_out = 0;
            exp_out = if f16_exp == 0 { 1 } else { exp_out + 1 };
        }
    }

    if inexact {
        flags = flags | FpFlags::NX;
    }
    // Underflow is raised when the rounded result is tiny (subnormal) AND
    // inexact. Per IEEE 754-2008, this is the "after rounding" definition
    // used by most FP units; RISC-V follows that convention.
    if f16_exp == 0 && inexact && exp_out == 0 {
        flags = flags | FpFlags::UF;
    }

    // Rounding could push us into infinity.
    if exp_out >= 0x1F {
        flags = flags | FpFlags::OF | FpFlags::NX;
        return (overflow_result(sign_u16, rm), flags);
    }

    let result = (sign_u16 << 15) | ((exp_out as u16) << 10) | (mant_out as u16);
    (result, flags)
}

/// Produces the correct f16 result for an overflow under the given
/// rounding mode: either ±inf or the max finite magnitude (0x7BFF).
#[inline]
const fn overflow_result(sign_u16: u16, rm: RoundingMode) -> u16 {
    let inf = (sign_u16 << 15) | 0x7C00;
    let max_finite = (sign_u16 << 15) | 0x7BFF;
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
