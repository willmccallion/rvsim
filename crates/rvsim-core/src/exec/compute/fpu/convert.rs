//! Float-to-integer conversion with RISC-V saturation and rounding to an
//! integral float.

use crate::isa::bits::sign_extend;
use crate::isa::op::AluOp;

use crate::isa::fp::{FpFlags, RoundingMode};

/// The integer an `fcvt` from floating point produces: its width and
/// signedness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntTarget {
    bits: u32,
    is_signed: bool,
}

impl IntTarget {
    /// A `bits`-bit signed integer.
    pub const fn signed(bits: u32) -> Self {
        Self { bits, is_signed: true }
    }

    /// A `bits`-bit unsigned integer.
    pub const fn unsigned(bits: u32) -> Self {
        Self { bits, is_signed: false }
    }

    /// The target of `op`, when it converts floating point to an integer.
    pub const fn of(op: AluOp) -> Option<Self> {
        match op {
            AluOp::FCvtWS => Some(Self::signed(32)),
            AluOp::FCvtWUS => Some(Self::unsigned(32)),
            AluOp::FCvtLS => Some(Self::signed(64)),
            AluOp::FCvtLUS => Some(Self::unsigned(64)),
            _ => None,
        }
    }

    /// Its width in bits.
    pub const fn bits(self) -> u32 {
        self.bits
    }

    /// The bits a value of the type occupies.
    const fn mask(self) -> u64 {
        u64::MAX >> (64 - self.bits)
    }

    /// The integral values that convert without overflow: from the first
    /// up to but excluding the second.
    const fn range(self) -> (f64, f64) {
        if self.is_signed {
            (-two_to_the(self.bits - 1), two_to_the(self.bits - 1))
        } else {
            (0.0, two_to_the(self.bits))
        }
    }

    /// The largest value's bits.
    const fn max(self) -> u64 {
        if self.is_signed { self.mask() >> 1 } else { self.mask() }
    }

    /// The smallest value's bits.
    const fn min(self) -> u64 {
        if self.is_signed { (self.mask() >> 1) + 1 } else { 0 }
    }

    /// The bits of `integral`, a value within [`Self::range`].
    const fn bits_of(self, integral: f64) -> u64 {
        let wide = if self.is_signed { integral as i64 as u64 } else { integral as u64 };
        wide & self.mask()
    }
}

/// 2 to the power `exponent`, exactly.
const fn two_to_the(exponent: u32) -> f64 {
    f64::from_bits((1023 + exponent as u64) << 52)
}

/// Converts `value` to the integer `target` under `rm`, saturating as
/// RISC-V does: NaN gives the maximum, and an out-of-range result raises
/// NV, which takes priority over NX. The result is the integer's bits at
/// the target's width, zero-extended.
pub fn fp_to_int(value: f64, target: IntTarget, rm: RoundingMode) -> (u64, FpFlags) {
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
    (target.bits_of(rounded), flags)
}

/// [`fp_to_int`] as RV64 writes the result to an integer register: a
/// 32-bit result sign-extended, signed or not.
pub(super) fn fp_to_int_register(
    value: f64,
    target: IntTarget,
    rm: RoundingMode,
) -> (u64, FpFlags) {
    let (bits, flags) = fp_to_int(value, target, rm);
    (sign_extend(bits, target.bits()) as u64, flags)
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
