//! Exact IEEE 754 arithmetic on finite operands, rounded in software.
//!
//! An operation's exact result is held as an [`Exact`]: a sign, an integer
//! significand, a power-of-two exponent and a sticky bit standing for any
//! nonzero bits below the significand. [`round`] rounds it to a binary16,
//! binary32 or binary64 in any RISC-V rounding mode, with tininess detected
//! after rounding as RISC-V requires. The host FPU lacks RMM (ties away from
//! zero), so RMM results come from here.

use super::host::{clear_host_fp_flags, read_host_fp_flags};
use crate::isa::fp::{FpFlags, RoundingMode};
use crate::isa::op::AluOp;

/// An IEEE 754 binary format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// binary16.
    Half,
    /// binary32.
    Single,
    /// binary64.
    Double,
}

impl Format {
    /// Significand bits, the hidden bit included.
    const fn precision(self) -> i32 {
        match self {
            Self::Half => 11,
            Self::Single => 24,
            Self::Double => 53,
        }
    }

    const fn exponent_bits(self) -> u32 {
        match self {
            Self::Half => 5,
            Self::Single => 8,
            Self::Double => 11,
        }
    }

    const fn bias(self) -> i32 {
        (1 << (self.exponent_bits() - 1)) - 1
    }

    /// The unbiased exponent of the smallest normal number.
    const fn min_exponent(self) -> i32 {
        1 - self.bias()
    }

    /// The unbiased exponent of the largest finite number.
    const fn max_exponent(self) -> i32 {
        self.bias()
    }

    const fn fraction_bits(self) -> u32 {
        (self.precision() - 1) as u32
    }

    const fn sign_bit(self) -> u64 {
        1 << (self.fraction_bits() + self.exponent_bits())
    }

    const fn infinity(self) -> u64 {
        ((1 << self.exponent_bits()) - 1) << self.fraction_bits()
    }

    const fn largest_finite(self) -> u64 {
        self.infinity() - 1
    }
}

/// A finite value: `(-1)^negative × (significand + f) × 2^exponent`, where
/// `f` is in (0, 1) when `sticky` and 0 otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exact {
    /// The sign.
    pub negative: bool,
    /// The integer significand.
    pub significand: u128,
    /// The power of two the significand is scaled by.
    pub exponent: i32,
    /// Whether nonzero bits lie below the significand.
    pub sticky: bool,
}

impl Exact {
    /// The value of the finite encoding `bits` in `format`.
    #[must_use]
    pub const fn of(bits: u64, format: Format) -> Self {
        let fraction_bits = format.fraction_bits();
        let exponent_field = ((bits >> fraction_bits) & ((1 << format.exponent_bits()) - 1)) as i32;
        let fraction = (bits & ((1 << fraction_bits) - 1)) as u128;
        let negative = bits & format.sign_bit() != 0;
        let (significand, exponent) = if exponent_field == 0 {
            (fraction, format.min_exponent() - fraction_bits as i32)
        } else {
            (fraction | (1 << fraction_bits), exponent_field - format.bias() - fraction_bits as i32)
        };
        Self { negative, significand, exponent, sticky: false }
    }

    /// The value of a finite `value`.
    #[must_use]
    pub const fn of_f32(value: f32) -> Self {
        Self::of(value.to_bits() as u64, Format::Single)
    }

    /// The value of a finite `value`.
    #[must_use]
    pub const fn of_f64(value: f64) -> Self {
        Self::of(value.to_bits(), Format::Double)
    }

    /// The value of an integer.
    #[must_use]
    pub const fn of_integer(value: i128) -> Self {
        Self { negative: value < 0, significand: value.unsigned_abs(), exponent: 0, sticky: false }
    }

    /// The value of an unsigned integer.
    #[must_use]
    pub const fn of_unsigned(value: u128) -> Self {
        Self { negative: false, significand: value, exponent: 0, sticky: false }
    }

    const fn is_zero(self) -> bool {
        self.significand == 0 && !self.sticky
    }

    /// The value with its sign flipped.
    #[must_use]
    pub const fn negated(self) -> Self {
        Self { negative: !self.negative, ..self }
    }

    /// The same value with the significand's top bit at bit `top`; low bits
    /// shifted out go to `sticky`.
    const fn with_top_bit_at(self, top: u32) -> Self {
        if self.significand == 0 {
            return self;
        }
        let current = self.significand.ilog2();
        if current < top {
            let shift = top - current;
            Self {
                significand: self.significand << shift,
                exponent: self.exponent - shift as i32,
                ..self
            }
        } else {
            let shift = current - top;
            let (significand, lost) = shift_right_sticky(self.significand, shift);
            Self {
                significand,
                exponent: self.exponent + shift as i32,
                sticky: self.sticky || lost,
                ..self
            }
        }
    }
}

/// `value >> shift`, and whether any bit shifted out was set.
const fn shift_right_sticky(value: u128, shift: u32) -> (u128, bool) {
    if shift >= 128 {
        (0, value != 0)
    } else if shift == 0 {
        (value, false)
    } else {
        (value >> shift, value & ((1 << shift) - 1) != 0)
    }
}

/// The exact sum. Both operands are exact (`sticky` clear); a zero operand
/// returns the other, and an exact cancellation returns a zero significand,
/// whose sign the caller decides.
#[must_use]
pub const fn add(x: Exact, y: Exact) -> Exact {
    if x.is_zero() {
        return y;
    }
    if y.is_zero() {
        return x;
    }
    // Top bits at 125 leave room for a carry, and a 53-bit or 106-bit
    // significand keeps at least 20 zero bits below it, so nothing is lost
    // until the exponents differ by enough that cancellation cannot be deep.
    let x = x.with_top_bit_at(125);
    let y = y.with_top_bit_at(125);
    let (big, small) = if x.exponent >= y.exponent { (x, y) } else { (y, x) };
    let (aligned, lost) =
        shift_right_sticky(small.significand, (big.exponent - small.exponent) as u32);
    if big.negative == small.negative {
        return Exact {
            negative: big.negative,
            significand: big.significand + aligned,
            exponent: big.exponent,
            sticky: lost,
        };
    }
    if big.significand >= aligned {
        // A lost fraction makes the subtrahend slightly larger.
        let difference = big.significand - aligned;
        let significand = if lost { difference - 1 } else { difference };
        return Exact { negative: big.negative, significand, exponent: big.exponent, sticky: lost };
    }
    Exact {
        negative: small.negative,
        significand: aligned - big.significand,
        exponent: big.exponent,
        sticky: false,
    }
}

/// The exact difference `x - y`.
#[must_use]
pub const fn sub(x: Exact, y: Exact) -> Exact {
    add(x, y.negated())
}

/// The exact product.
#[must_use]
pub const fn mul(x: Exact, y: Exact) -> Exact {
    Exact {
        negative: x.negative != y.negative,
        significand: x.significand * y.significand,
        exponent: x.exponent + y.exponent,
        sticky: false,
    }
}

/// The exact `x × y + z`.
#[must_use]
pub const fn mul_add(x: Exact, y: Exact, z: Exact) -> Exact {
    add(mul(x, y), z)
}

/// The quotient to more bits than any format keeps, with a sticky bit for a
/// nonzero remainder. Both operands are nonzero.
#[must_use]
pub const fn div(x: Exact, y: Exact) -> Exact {
    let x = x.with_top_bit_at(52);
    let y = y.with_top_bit_at(52);
    let dividend = x.significand << 74;
    Exact {
        negative: x.negative != y.negative,
        significand: dividend / y.significand,
        exponent: x.exponent - y.exponent - 74,
        sticky: !dividend.is_multiple_of(y.significand),
    }
}

/// The square root of a positive value to more bits than any format keeps,
/// with a sticky bit for a nonzero remainder.
#[must_use]
pub const fn sqrt(x: Exact) -> Exact {
    let x = x.with_top_bit_at(52);
    // An even exponent after shifting, and a radicand near 2^120.
    let shift: i32 = if (x.exponent - 66) % 2 == 0 { 66 } else { 67 };
    let radicand = x.significand << shift;
    let root = integer_sqrt(radicand);
    Exact {
        negative: false,
        significand: root,
        exponent: (x.exponent - shift) / 2,
        sticky: root * root != radicand,
    }
}

/// The largest integer whose square is at most `value`.
const fn integer_sqrt(value: u128) -> u128 {
    let mut root: u128 = 0;
    let mut bit: u128 = 1 << 126;
    let mut remainder = value;
    while bit > value {
        bit >>= 2;
    }
    while bit != 0 {
        if remainder >= root + bit {
            remainder -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root
}

/// The exact result of the scalar arithmetic `op` on `a`, `b` and `c`, the
/// fused multiply-add family's operands in the ISA's order; `None` for an
/// operation that does not round.
#[must_use]
pub const fn of_operation(op: AluOp, a: Exact, b: Exact, c: Exact) -> Option<Exact> {
    Some(match op {
        AluOp::FAdd => add(a, b),
        AluOp::FSub => sub(a, b),
        AluOp::FMul => mul(a, b),
        AluOp::FDiv => div(a, b),
        AluOp::FSqrt => sqrt(a),
        AluOp::FMAdd => mul_add(a, b, c),
        AluOp::FMSub => mul_add(a, b, c.negated()),
        AluOp::FNMAdd => mul_add(a.negated(), b, c.negated()),
        AluOp::FNMSub => mul_add(a.negated(), b, c),
        _ => return None,
    })
}

/// The host FPU has no RMM mode and rounds it as nearest-even, which can
/// differ on a tie. Given the flags the host raised for an operation and
/// the operation's exact result, the correctly rounded RMM result and its
/// flags, or `None` when `rm` is not RMM or the host's result was exact or
/// invalid. An inexact result implies finite operands, a nonzero divisor
/// and a positive square-root operand, which `exact` may rely on.
#[must_use]
pub fn rmm_correction(
    rm: RoundingMode,
    host_flags: FpFlags,
    format: Format,
    exact: impl FnOnce() -> Option<Exact>,
) -> Option<(u64, FpFlags)> {
    if rm != RoundingMode::Rmm
        || !host_flags.contains(FpFlags::NX)
        || host_flags.contains(FpFlags::NV)
    {
        return None;
    }
    exact().map(|value| round(value, format, rm))
}

/// Runs `host` on the host FPU, whose rounding mode the caller set for `rm`,
/// and returns its result and flags, rounding `exact` (the operation's exact
/// result) in software instead under RMM when the host's was inexact.
pub fn on_host_f32(
    rm: RoundingMode,
    host: impl FnOnce() -> f32,
    exact: impl FnOnce() -> Exact,
) -> (f32, FpFlags) {
    clear_host_fp_flags();
    let result = std::hint::black_box(host());
    let flags = read_host_fp_flags();
    match rmm_correction(rm, flags, Format::Single, || Some(exact())) {
        Some((bits, flags)) => (f32::from_bits(bits as u32), flags),
        None => (result, flags),
    }
}

/// [`on_host_f32`] for binary64.
pub fn on_host_f64(
    rm: RoundingMode,
    host: impl FnOnce() -> f64,
    exact: impl FnOnce() -> Exact,
) -> (f64, FpFlags) {
    clear_host_fp_flags();
    let result = std::hint::black_box(host());
    let flags = read_host_fp_flags();
    match rmm_correction(rm, flags, Format::Double, || Some(exact())) {
        Some((bits, flags)) => (f64::from_bits(bits), flags),
        None => (result, flags),
    }
}

/// The part of a value rounding discards: its top bit, and whether any bit
/// below that is set.
#[derive(Clone, Copy)]
struct Discarded {
    round: bool,
    sticky: bool,
}

impl Discarded {
    const fn is_inexact(self) -> bool {
        self.round || self.sticky
    }
}

/// Whether rounding increases the magnitude, given the sign and the kept
/// part's lowest bit `odd`.
const fn rounds_up(rm: RoundingMode, negative: bool, odd: bool, discarded: Discarded) -> bool {
    match rm {
        RoundingMode::Rne => discarded.round && (discarded.sticky || odd),
        RoundingMode::Rtz => false,
        RoundingMode::Rdn => negative && discarded.is_inexact(),
        RoundingMode::Rup => !negative && discarded.is_inexact(),
        RoundingMode::Rmm => discarded.round,
    }
}

/// `x` rounded to an integer multiple of `2^lsb_exponent`: the multiple, and
/// whether rounding discarded anything.
const fn round_to_multiple(x: Exact, lsb_exponent: i32, rm: RoundingMode) -> (u128, bool) {
    let shift = lsb_exponent - x.exponent;
    let (kept, round, sticky) = if shift <= 0 {
        (x.significand << (-shift) as u32, false, x.sticky)
    } else {
        let shift = shift as u32;
        let (kept, _) = shift_right_sticky(x.significand, shift);
        let round = shift <= 128 && (x.significand >> (shift - 1)) & 1 == 1;
        let (_, below_round) = shift_right_sticky(x.significand, shift.saturating_sub(1));
        let below = if shift >= 2 { below_round } else { false };
        (kept, round, below || x.sticky)
    };
    let discarded = Discarded { round, sticky };
    let up = rounds_up(rm, x.negative, kept & 1 == 1, discarded);
    (if up { kept + 1 } else { kept }, discarded.is_inexact())
}

/// The result an overflow in `rm` gives: infinity, or the largest finite
/// number when the mode rounds toward zero on that side.
const fn overflowed(format: Format, negative: bool, rm: RoundingMode) -> u64 {
    let sign = if negative { format.sign_bit() } else { 0 };
    let to_largest = match rm {
        RoundingMode::Rne | RoundingMode::Rmm => false,
        RoundingMode::Rtz => true,
        RoundingMode::Rdn => !negative,
        RoundingMode::Rup => negative,
    };
    sign | if to_largest { format.largest_finite() } else { format.infinity() }
}

/// `x` rounded to `format` in `rm`, with the flags rounding raises.
#[must_use]
pub const fn round(x: Exact, format: Format, rm: RoundingMode) -> (u64, FpFlags) {
    let sign = if x.negative { format.sign_bit() } else { 0 };
    if x.is_zero() {
        return (sign, FpFlags::NONE);
    }
    let x = if x.significand == 0 {
        // Only a sticky fraction: below every bit any format keeps here.
        Exact { significand: 1, exponent: x.exponent - 128, ..x }
    } else {
        x
    };
    let precision = format.precision();
    let top = x.exponent + x.significand.ilog2() as i32;
    let min_lsb = format.min_exponent() - (precision - 1);
    let unbounded_lsb = top - (precision - 1);
    let lsb = if unbounded_lsb > min_lsb { unbounded_lsb } else { min_lsb };
    let (kept, inexact) = round_to_multiple(x, lsb, rm);
    if kept == 0 {
        return (sign, underflow_flags(true, inexact));
    }
    let kept_top = lsb + kept.ilog2() as i32;
    if kept_top > format.max_exponent() {
        return (overflowed(format, x.negative, rm), FpFlags::OF.union(FpFlags::NX));
    }
    let tiny = tiny_after_rounding(x, unbounded_lsb, format, rm);
    let bits = if kept_top >= format.min_exponent() {
        let normalized = kept >> (kept_top - lsb - (precision - 1)) as u32;
        let fraction = (normalized as u64) & ((1 << format.fraction_bits()) - 1);
        let exponent_field = (kept_top + format.bias()) as u64;
        (exponent_field << format.fraction_bits()) | fraction
    } else {
        kept as u64
    };
    (sign | bits, underflow_flags(tiny, inexact))
}

/// Whether `x` rounded with an unbounded exponent range is below the
/// smallest normal number: RISC-V detects tininess after rounding.
const fn tiny_after_rounding(
    x: Exact,
    unbounded_lsb: i32,
    format: Format,
    rm: RoundingMode,
) -> bool {
    let (kept, _) = round_to_multiple(x, unbounded_lsb, rm);
    unbounded_lsb + (kept.ilog2() as i32) < format.min_exponent()
}

const fn underflow_flags(tiny: bool, inexact: bool) -> FpFlags {
    match (tiny, inexact) {
        (true, true) => FpFlags::UF.union(FpFlags::NX),
        (_, true) => FpFlags::NX,
        _ => FpFlags::NONE,
    }
}
