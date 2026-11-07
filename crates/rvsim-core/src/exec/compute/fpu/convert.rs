//! Float-to-integer conversion with RISC-V saturation and rounding to an
//! integral float.

use crate::isa::bits::sext_word;
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

/// The integer type an `fcvt` from floating point produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IntTarget {
    I32,
    U32,
    I64,
    U64,
}

impl IntTarget {
    /// The target of `op`, when it converts floating point to an integer.
    pub(super) const fn of(op: AluOp) -> Option<Self> {
        match op {
            AluOp::FCvtWS => Some(Self::I32),
            AluOp::FCvtWUS => Some(Self::U32),
            AluOp::FCvtLS => Some(Self::I64),
            AluOp::FCvtLUS => Some(Self::U64),
            _ => None,
        }
    }

    /// The largest value, as the sign-extended register image RISC-V
    /// writes for the type.
    const fn max(self) -> u64 {
        match self {
            Self::I32 => i32::MAX as i64 as u64,
            Self::U32 => sext_word(u32::MAX),
            Self::I64 => i64::MAX as u64,
            Self::U64 => u64::MAX,
        }
    }

    /// The smallest value, as the register image.
    const fn min(self) -> u64 {
        match self {
            Self::I32 => i32::MIN as i64 as u64,
            Self::U32 | Self::U64 => 0,
            Self::I64 => i64::MIN as u64,
        }
    }

    /// The integral values that convert without overflow: from the first
    /// up to but excluding the second.
    const fn range(self) -> (f64, f64) {
        match self {
            Self::I32 => (I32_MIN_F64, I32_MAX_P1_F64),
            Self::U32 => (0.0, U32_MAX_P1_F64),
            Self::I64 => (I64_MIN_F64, I64_MAX_P1_F64),
            Self::U64 => (0.0, U64_MAX_P1_F64),
        }
    }

    /// The register image of `integral`, a value within [`Self::range`].
    const fn image(self, integral: f64) -> u64 {
        match self {
            Self::I32 => integral as i32 as i64 as u64,
            Self::U32 => sext_word(integral as u32),
            Self::I64 => integral as i64 as u64,
            Self::U64 => integral as u64,
        }
    }
}

/// Converts `value` to the integer `target` under `rm`, saturating as
/// RISC-V does: NaN gives the maximum, and an out-of-range result raises
/// NV, which takes priority over NX.
pub(super) fn fp_to_int(value: f64, target: IntTarget, rm: RoundingMode) -> (u64, FpFlags) {
    if value.is_nan() {
        return (target.max(), FpFlags::NV);
    }
    let rounded = round_to_integer(value, rm);
    let (low, high) = target.range();
    if rounded < low {
        return (target.min(), FpFlags::NV);
    }
    if rounded >= high {
        return (target.max(), FpFlags::NV);
    }
    let flags = if value == rounded { FpFlags::NONE } else { FpFlags::NX };
    (target.image(rounded), flags)
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
