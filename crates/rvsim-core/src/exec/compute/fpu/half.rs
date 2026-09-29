//! Half-precision (Zfh) helpers: NaN-boxing, f16↔f32↔f64 conversions with
//! IEEE 754 rounding, classification, and signaling-NaN detection.
//!
//! The host has no native f16 type, so half-precision values are represented
//! as `u16` bit patterns. Arithmetic is performed by upcasting to `f64`
//! (lossless for add/sub/mul/fma of f16 inputs) and then software-rounding
//! back to f16 with the RISC-V rounding mode.

use super::convert::fp_to_int_convert;
use super::host::{restore_host_round_mode, set_host_round_mode};
use super::nan_handling::{fmax_f32, fmin_f32, is_snan_f32, is_snan_f64, unbox_f32};
use crate::isa::fp::{FpFlags, RoundingMode};
use crate::isa::op::AluOp;

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
    if (val & F16_NAN_BOX_MASK) == F16_NAN_BOX_MASK { val as u16 } else { CANONICAL_NAN_F16 }
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
pub const fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1F) as u32;
    let mant = (h & 0x3FF) as u32;

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
            m &= 0x3FF;
            let new_exp = (e + 127) as u32;
            (sign << 31) | (new_exp << 23) | (m << 13)
        }
    } else if exp == 0x1F {
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
        return ((sign_u16 << 15) | 0x7C00, FpFlags::NONE);
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
            RoundingMode::Rdn if sign_u16 == 1 => (sign_u16 << 15) | 1,
            RoundingMode::Rup if sign_u16 == 0 => (sign_u16 << 15) | 1,
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
    let round_bit = (sig >> (shift - 1)) & 1;
    let sticky_mask = (1u64 << (shift - 1)) - 1;
    let sticky = (sig & sticky_mask) != 0;

    let base_mant: u32 =
        if f16_exp > 0 { (mant_shifted as u32) & 0x3FF } else { mant_shifted as u32 };

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
            mant_out = 0;
            exp_out = if f16_exp == 0 { 1 } else { exp_out + 1 };
        }
    }

    if inexact {
        flags = flags | FpFlags::NX;
    }
    // IEEE 754-2008 "after rounding" UF: tiny (subnormal) AND inexact.
    if f16_exp == 0 && inexact && exp_out == 0 {
        flags = flags | FpFlags::UF;
    }

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

/// Executes a half-precision (Zfh) floating-point operation and returns
/// `(result, flags)`.
///
/// The host has no native f16 type, so half-precision arithmetic is
/// performed by upcasting operands to f64 (lossless for f16 inputs on
/// add/sub/mul/fma; near-exact for div/sqrt which still have >50 bits
/// of working precision). The f64 result is then software-rounded to
/// f16 with the given RISC-V rounding mode by [`half::f64_to_f16`],
/// which also accumulates the IEEE 754 exception flags.
pub(super) fn execute_f16(op: AluOp, a: u64, b: u64, c: u64, rm: RoundingMode) -> (u64, FpFlags) {
    // Set the host FPU rounding mode for the intermediate f64 step. This
    // matters for edge cases like `a + (-a)` under RDN where the host
    // must produce `-0` — the software round-to-f16 cannot recover a
    // negative-zero result from a host-RNE `+0`.
    let saved_round = set_host_round_mode(rm);
    let result = execute_f16_inner(op, a, b, c, rm);
    restore_host_round_mode(saved_round);
    result
}

pub(super) fn execute_f16_inner(
    op: AluOp,
    a: u64,
    b: u64,
    c: u64,
    rm: RoundingMode,
) -> (u64, FpFlags) {
    let ha = unbox_f16(a);
    let hb = unbox_f16(b);
    let hc = unbox_f16(c);

    // Helper: round an f64 arith result to f16 and merge any extra flags.
    let arith = |val: f64, extra: FpFlags| {
        let (bits, flags) = f64_to_f16(val, rm);
        (box_f16(bits), flags | extra)
    };

    match op {
        AluOp::FAdd => {
            let fa = f16_to_f32(ha) as f64;
            let fb = f16_to_f32(hb) as f64;
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            arith(fa + fb, nv)
        }
        AluOp::FSub => {
            let fa = f16_to_f32(ha) as f64;
            let fb = f16_to_f32(hb) as f64;
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            arith(fa - fb, nv)
        }
        AluOp::FMul => {
            let fa = f16_to_f32(ha) as f64;
            let fb = f16_to_f32(hb) as f64;
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            arith(fa * fb, nv)
        }
        AluOp::FDiv => {
            let fa = f16_to_f32(ha) as f64;
            let fb = f16_to_f32(hb) as f64;
            let mut extra = FpFlags::NONE;
            if is_snan_f16(ha) || is_snan_f16(hb) {
                extra = extra | FpFlags::NV;
            }
            if fb == 0.0 && fa.is_finite() && fa != 0.0 {
                extra = extra | FpFlags::DZ;
            } else if fb == 0.0 && fa == 0.0 {
                extra = extra | FpFlags::NV; // 0/0 → NaN, invalid
            } else if fa.is_infinite() && fb.is_infinite() {
                extra = extra | FpFlags::NV; // inf/inf → NaN, invalid
            }
            arith(fa / fb, extra)
        }
        AluOp::FSqrt => {
            let fa = f16_to_f32(ha) as f64;
            let mut extra = FpFlags::NONE;
            if is_snan_f16(ha) {
                extra = extra | FpFlags::NV;
            } else if fa < 0.0 {
                // sqrt of strictly negative → invalid (sqrt(-0) is fine)
                extra = extra | FpFlags::NV;
            }
            arith(fa.sqrt(), extra)
        }
        AluOp::FMAdd => {
            let fa = f16_to_f32(ha) as f64;
            let fb = f16_to_f32(hb) as f64;
            let fc = f16_to_f32(hc) as f64;
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) || is_snan_f16(hc) {
                FpFlags::NV
            } else {
                FpFlags::NONE
            };
            arith(fa.mul_add(fb, fc), nv)
        }
        AluOp::FMSub => {
            let fa = f16_to_f32(ha) as f64;
            let fb = f16_to_f32(hb) as f64;
            let fc = f16_to_f32(hc) as f64;
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) || is_snan_f16(hc) {
                FpFlags::NV
            } else {
                FpFlags::NONE
            };
            arith(fa.mul_add(fb, -fc), nv)
        }
        AluOp::FNMAdd => {
            let fa = f16_to_f32(ha) as f64;
            let fb = f16_to_f32(hb) as f64;
            let fc = f16_to_f32(hc) as f64;
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) || is_snan_f16(hc) {
                FpFlags::NV
            } else {
                FpFlags::NONE
            };
            arith((-fa).mul_add(fb, -fc), nv)
        }
        AluOp::FNMSub => {
            let fa = f16_to_f32(ha) as f64;
            let fb = f16_to_f32(hb) as f64;
            let fc = f16_to_f32(hc) as f64;
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) || is_snan_f16(hc) {
                FpFlags::NV
            } else {
                FpFlags::NONE
            };
            arith((-fa).mul_add(fb, fc), nv)
        }

        AluOp::FSgnJ => (box_f16((ha & 0x7FFF) | (hb & 0x8000)), FpFlags::NONE),
        AluOp::FSgnJN => (box_f16((ha & 0x7FFF) | (!hb & 0x8000)), FpFlags::NONE),
        AluOp::FSgnJX => (box_f16(ha ^ (hb & 0x8000)), FpFlags::NONE),

        AluOp::FMin | AluOp::FMax => {
            let fa = f16_to_f32(ha);
            let fb = f16_to_f32(hb);
            let mut flags = FpFlags::NONE;
            if is_snan_f16(ha) || is_snan_f16(hb) {
                flags = flags | FpFlags::NV;
            }
            let r = if matches!(op, AluOp::FMin) { fmin_f32(fa, fb) } else { fmax_f32(fa, fb) };
            if r.is_nan() {
                return (box_f16(CANONICAL_NAN_F16), flags);
            }
            let (bits, _) = f64_to_f16(r as f64, RoundingMode::Rne);
            (box_f16(bits), flags)
        }

        AluOp::FEq => {
            let nv = if is_snan_f16(ha) || is_snan_f16(hb) { FpFlags::NV } else { FpFlags::NONE };
            let fa = f16_to_f32(ha);
            let fb = f16_to_f32(hb);
            ((fa == fb) as u64, nv)
        }
        AluOp::FLt => {
            let fa = f16_to_f32(ha);
            let fb = f16_to_f32(hb);
            let nv = if fa.is_nan() || fb.is_nan() { FpFlags::NV } else { FpFlags::NONE };
            ((fa < fb) as u64, nv)
        }
        AluOp::FLe => {
            let fa = f16_to_f32(ha);
            let fb = f16_to_f32(hb);
            let nv = if fa.is_nan() || fb.is_nan() { FpFlags::NV } else { FpFlags::NONE };
            ((fa <= fb) as u64, nv)
        }

        AluOp::FClass => (classify_f16(ha), FpFlags::NONE),

        // fmv.x.h: RAW bit-cast of the low 16 bits of the f register,
        // sign-extended to XLEN. Per spec, this is NOT NaN-boxing-aware
        // — `unbox_f16` would incorrectly canonicalize a non-NaN-boxed
        // value so we read from `a` directly instead.
        AluOp::FMvToX => (((a as i16) as i64) as u64, FpFlags::NONE),
        // fmv.h.x: take the low 16 bits of the integer operand, NaN-box.
        AluOp::FMvToF => (box_f16(a as u16), FpFlags::NONE),

        // f16 → integer conversions. Upcast f16 to f64, then use the
        // existing integer-range rounding logic (same as f32/f64 ints).
        AluOp::FCvtWS | AluOp::FCvtWUS | AluOp::FCvtLS | AluOp::FCvtLUS => {
            let val = f16_to_f32(ha) as f64;
            fp_to_int_convert(op, val, rm)
        }

        // Float → f16 conversions (target = half).
        AluOp::FCvtHS => {
            // fcvt.h.s: source is NaN-boxed f32 in `a`.
            let fval = unbox_f32(a);
            let nv = if is_snan_f32(fval) { FpFlags::NV } else { FpFlags::NONE };
            let (bits, flags) = f64_to_f16(fval as f64, rm);
            (box_f16(bits), flags | nv)
        }
        AluOp::FCvtHD => {
            // fcvt.h.d: source is f64 in `a`.
            let fval = f64::from_bits(a);
            let nv = if is_snan_f64(fval) { FpFlags::NV } else { FpFlags::NONE };
            let (bits, flags) = f64_to_f16(fval, rm);
            (box_f16(bits), flags | nv)
        }

        // integer → f16 conversions.
        AluOp::FCvtSW => {
            let r = (a as i32) as f64;
            let (bits, flags) = f64_to_f16(r, rm);
            (box_f16(bits), flags)
        }
        AluOp::FCvtSWU => {
            let r = (a as u32) as f64;
            let (bits, flags) = f64_to_f16(r, rm);
            (box_f16(bits), flags)
        }
        AluOp::FCvtSL => {
            let r = (a as i64) as f64;
            let (bits, flags) = f64_to_f16(r, rm);
            (box_f16(bits), flags)
        }
        AluOp::FCvtSLU => {
            let r = a as f64;
            let (bits, flags) = f64_to_f16(r, rm);
            (box_f16(bits), flags)
        }

        _ => (0, FpFlags::NONE),
    }
}
