//! Float-to-integer conversion with RISC-V saturation and rounding to an
//! integral float.

use crate::isa::op::AluOp;

use crate::isa::fp::{FpFlags, RoundingMode};

/// `i32::MAX` + 1 as f64 (2^31). Values >= this overflow i32.
pub(super) const I32_MAX_P1_F64: f64 = (i32::MAX as f64) + 1.0;

/// `i32::MIN` as f64 (-2^31). Values < this overflow i32.
pub(super) const I32_MIN_F64: f64 = i32::MIN as f64;

/// `u32::MAX` + 1 as f64 (2^32). Values >= this overflow u32.
pub(super) const U32_MAX_P1_F64: f64 = (u32::MAX as f64) + 1.0;

/// `i64::MAX` + 1 as f64 (2^63). Values >= this overflow i64.
pub(super) const I64_MAX_P1_F64: f64 = 9223372036854775808.0; // 2^63 exactly
/// `i64::MIN` as f64 (-2^63). Values < this overflow i64.
pub(super) const I64_MIN_F64: f64 = i64::MIN as f64;

/// `u64::MAX` + 1 as f64 (2^64). Values >= this overflow u64.
pub(super) const U64_MAX_P1_F64: f64 = 18446744073709551616.0; // 2^64 exactly

// ---- RISC-V float-to-integer conversion helpers ----
// Rust's `f as i32` saturates correctly for ±Inf and out-of-range values,
// but produces 0 for NaN.  RISC-V requires positive-max for NaN.

/// Convert f64 value to i32 per RISC-V spec (NaN → `INT32_MAX`).
#[inline]
pub(super) const fn f64_to_i32_rv(v: f64) -> i32 {
    if v.is_nan() {
        i32::MAX
    } else {
        v as i32 // Rust saturates: +Inf→MAX, -Inf→MIN, out-of-range→saturated
    }
}

/// Convert f64 value to u32 per RISC-V spec (NaN → `UINT32_MAX`).
#[inline]
pub(super) const fn f64_to_u32_rv(v: f64) -> u32 {
    if v.is_nan() { u32::MAX } else { v as u32 }
}

/// Convert f64 value to i64 per RISC-V spec (NaN → `INT64_MAX`).
#[inline]
pub(super) const fn f64_to_i64_rv(v: f64) -> i64 {
    if v.is_nan() { i64::MAX } else { v as i64 }
}

/// Convert f64 value to u64 per RISC-V spec (NaN → `UINT64_MAX`).
#[inline]
pub(super) const fn f64_to_u64_rv(v: f64) -> u64 {
    if v.is_nan() { u64::MAX } else { v as u64 }
}

/// Converts an f64 value to a RISC-V integer result with the given
/// rounding mode. Used by [`execute_f16`] for f16→int conversions;
/// mirrors the logic inline in [`execute_full_rm`] for f32/f64→int.
pub(super) fn fp_to_int_convert(op: AluOp, val: f64, rm: RoundingMode) -> (u64, FpFlags) {
    let mut flags = FpFlags::NONE;
    if val.is_nan() {
        flags = flags | FpFlags::NV;
        let result = match op {
            AluOp::FCvtWS => i32::MAX as i64 as u64,
            AluOp::FCvtWUS => u32::MAX as i32 as i64 as u64,
            AluOp::FCvtLS => i64::MAX as u64,
            AluOp::FCvtLUS => u64::MAX,
            _ => 0,
        };
        return (result, flags);
    }
    if val.is_infinite() {
        flags = flags | FpFlags::NV;
        let result = match op {
            AluOp::FCvtWS => {
                if val > 0.0 {
                    i32::MAX as i64 as u64
                } else {
                    i32::MIN as i64 as u64
                }
            }
            AluOp::FCvtWUS => {
                if val > 0.0 {
                    u32::MAX as i32 as i64 as u64
                } else {
                    0
                }
            }
            AluOp::FCvtLS => {
                if val > 0.0 {
                    i64::MAX as u64
                } else {
                    i64::MIN as u64
                }
            }
            AluOp::FCvtLUS => {
                if val > 0.0 {
                    u64::MAX
                } else {
                    0
                }
            }
            _ => 0,
        };
        return (result, flags);
    }

    let rounded = round_to_integer(val, rm);
    let inexact = val != rounded;

    let (overflow, result) = match op {
        AluOp::FCvtWS => {
            if (I32_MIN_F64..I32_MAX_P1_F64).contains(&rounded) {
                (false, rounded as i32 as i64 as u64)
            } else {
                (true, if rounded > 0.0 { i32::MAX } else { i32::MIN } as i64 as u64)
            }
        }
        AluOp::FCvtWUS => {
            if (0.0..U32_MAX_P1_F64).contains(&rounded) {
                (false, rounded as u32 as i32 as i64 as u64)
            } else {
                (true, if rounded > 0.0 { u32::MAX as i32 as i64 as u64 } else { 0 })
            }
        }
        AluOp::FCvtLS => {
            if (I64_MIN_F64..I64_MAX_P1_F64).contains(&rounded) {
                (false, rounded as i64 as u64)
            } else {
                (true, if rounded > 0.0 { i64::MAX } else { i64::MIN } as u64)
            }
        }
        AluOp::FCvtLUS => {
            if rounded < 0.0 {
                (true, 0u64)
            } else if rounded >= U64_MAX_P1_F64 {
                (true, u64::MAX)
            } else {
                (false, rounded as u64)
            }
        }
        _ => (false, 0),
    };

    if overflow {
        flags = flags | FpFlags::NV;
    } else if inexact {
        flags = flags | FpFlags::NX;
    }
    (result, flags)
}

/// Rounds an f64 value to an integer using the specified RISC-V rounding mode.
pub(super) const fn round_to_integer(val: f64, rm: RoundingMode) -> f64 {
    match rm {
        RoundingMode::Rne => {
            // Round to nearest, ties to even — IEEE 754 default.
            // Rust's f64::round_ties_even is available since 1.77.
            val.round_ties_even()
        }
        RoundingMode::Rtz => val.trunc(),
        RoundingMode::Rdn => val.floor(),
        RoundingMode::Rup => val.ceil(),
        RoundingMode::Rmm => {
            // Round to nearest, ties to max magnitude (away from zero).
            // f64::round() does ties-away-from-zero, which is RMM.
            val.round()
        }
    }
}
