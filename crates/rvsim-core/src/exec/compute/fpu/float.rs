//! The floating-point formats the scalar and vector units compute in, so
//! that each operation is written once: f16 in software, f32 and f64 on
//! the host.
//!
//! Operations that round run on the host FPU, whose rounding mode the
//! caller sets for `rm`: the host formats directly, and [`SoftF16`] for the
//! f64 step whose zero result's sign the mode decides.

use super::exact::{self, Exact, on_host_f32, on_host_f64};
use super::half::{
    CANONICAL_NAN_F16, F16_SIGN, f16_to_f32, f64_to_f16, fused_is_invalid, is_snan_f16,
    product_is_invalid, sum_is_invalid,
};
use super::host::{clear_host_fp_flags, read_host_fp_flags};
use super::nan_handling::{fmax_f32, fmax_f64, fmin_f32, fmin_f64, is_snan_f32, is_snan_f64};
use crate::isa::fp::{FpFlags, FpFormat, RoundingMode};
use crate::isa::op::{IntWidth, Signedness};
use std::hint::black_box;
use std::ops::Neg;

/// A floating-point format: its values, the operations on them rounded
/// under a rounding mode with the IEEE flags they raise, and its image in a
/// 64-bit FP register.
pub trait Float: Copy + Neg<Output = Self> {
    /// Its format.
    const FORMAT: FpFormat;
    /// Its width in bits.
    const BITS: u32;
    /// Its sign bit.
    const SIGN_BIT: u64 = 1 << (Self::BITS - 1);
    /// The canonical quiet NaN.
    const CANONICAL_NAN: Self;

    /// The value the low [`Self::BITS`] bits of `bits` encode.
    fn from_raw_bits(bits: u64) -> Self;
    /// Its encoding, zero-extended.
    fn raw_bits(self) -> u64;
    /// Its value as an f64, which every format widens to exactly.
    fn to_f64(self) -> f64;
    /// `value`, which the format holds exactly: a narrower format's value
    /// or a NaN.
    fn from_f64_exact(value: f64) -> Self;
    /// Whether it is a NaN.
    fn is_nan(self) -> bool;
    /// Whether it is a signaling NaN.
    fn is_signaling_nan(self) -> bool;

    /// `self + other`.
    fn add(self, other: Self, rm: RoundingMode) -> (Self, FpFlags);
    /// `self - other`.
    fn sub(self, other: Self, rm: RoundingMode) -> (Self, FpFlags);
    /// `self × other`.
    fn mul(self, other: Self, rm: RoundingMode) -> (Self, FpFlags);
    /// `self / other`.
    fn div(self, other: Self, rm: RoundingMode) -> (Self, FpFlags);
    /// The square root of `self`.
    fn sqrt(self, rm: RoundingMode) -> (Self, FpFlags);
    /// `self × b + c`, rounded once.
    fn mul_add(self, b: Self, c: Self, rm: RoundingMode) -> (Self, FpFlags);
    /// The minimum (IEEE 754-2008 minNum, a canonical NaN only when both
    /// are NaN).
    fn min(self, other: Self) -> Self;
    /// The maximum, as [`Self::min`].
    fn max(self, other: Self) -> Self;
    /// The integer `value` of the given width and signedness, taken from
    /// the low bits of `value`.
    fn from_int(value: u64, kind: (IntWidth, Signedness), rm: RoundingMode) -> (Self, FpFlags);
    /// `value` rounded to the format.
    fn from_f64_rounded(value: f64, rm: RoundingMode) -> (Self, FpFlags);

    /// Itself, or the canonical NaN if it is a NaN: an arithmetic result.
    fn canonical(self) -> Self {
        if self.is_nan() { Self::CANONICAL_NAN } else { self }
    }

    /// Its image in a 64-bit FP register: NaN-boxed when narrower.
    fn to_register(self) -> u64 {
        let bits = self.raw_bits();
        if Self::BITS == u64::BITS { bits } else { bits | (u64::MAX << Self::BITS) }
    }

    /// The value a 64-bit FP register holds. A narrower value that is not
    /// NaN-boxed reads as the canonical NaN (RISC-V spec §12.2).
    fn from_register(register: u64) -> Self {
        let is_boxed = Self::BITS == u64::BITS || register >> Self::BITS == u64::MAX >> Self::BITS;
        if is_boxed { Self::from_raw_bits(register) } else { Self::CANONICAL_NAN }
    }
}

/// An f16, computed through f64, which holds every f16 sum, difference and
/// product exactly, and rounded to f16 in software.
#[derive(Clone, Copy, Debug)]
pub struct SoftF16(u16);

impl SoftF16 {
    /// `value` rounded to f16 under `rm`, with the rounding's flags and `extra`.
    fn round(value: f64, rm: RoundingMode, extra: FpFlags) -> (Self, FpFlags) {
        let (bits, flags) = f64_to_f16(value, rm);
        (Self(bits), flags | extra)
    }

    /// NV when either operand is a signaling NaN or the operation is invalid.
    fn invalid(self, other: Self, is_invalid_operation: bool) -> FpFlags {
        let signaling = self.is_signaling_nan() || other.is_signaling_nan();
        if signaling || is_invalid_operation { FpFlags::NV } else { FpFlags::NONE }
    }

    /// The f16 that `value`, an f16 widened to f32 or a NaN, is.
    fn of_widened(value: f32) -> Self {
        if value.is_nan() {
            return Self::CANONICAL_NAN;
        }
        Self::from_f64_exact(f64::from(value))
    }
}

impl Neg for SoftF16 {
    type Output = Self;

    fn neg(self) -> Self {
        Self(self.0 ^ F16_SIGN)
    }
}

impl Float for SoftF16 {
    const FORMAT: FpFormat = FpFormat::Half;
    const BITS: u32 = 16;
    const CANONICAL_NAN: Self = Self(CANONICAL_NAN_F16);

    fn from_raw_bits(bits: u64) -> Self {
        Self(bits as u16)
    }

    fn raw_bits(self) -> u64 {
        u64::from(self.0)
    }

    fn to_f64(self) -> f64 {
        f64::from(f16_to_f32(self.0))
    }

    fn from_f64_exact(value: f64) -> Self {
        Self(f64_to_f16(value, RoundingMode::Rne).0)
    }

    fn is_nan(self) -> bool {
        f16_to_f32(self.0).is_nan()
    }

    fn is_signaling_nan(self) -> bool {
        is_snan_f16(self.0)
    }

    fn add(self, other: Self, rm: RoundingMode) -> (Self, FpFlags) {
        let (a, b) = (self.to_f64(), other.to_f64());
        Self::round(a + b, rm, self.invalid(other, sum_is_invalid(a, b)))
    }

    fn sub(self, other: Self, rm: RoundingMode) -> (Self, FpFlags) {
        let (a, b) = (self.to_f64(), other.to_f64());
        Self::round(a - b, rm, self.invalid(other, sum_is_invalid(a, -b)))
    }

    fn mul(self, other: Self, rm: RoundingMode) -> (Self, FpFlags) {
        let (a, b) = (self.to_f64(), other.to_f64());
        Self::round(a * b, rm, self.invalid(other, product_is_invalid(a, b)))
    }

    fn div(self, other: Self, rm: RoundingMode) -> (Self, FpFlags) {
        let (a, b) = (self.to_f64(), other.to_f64());
        let extra = if self.is_signaling_nan() || other.is_signaling_nan() {
            FpFlags::NV
        } else if b == 0.0 && a != 0.0 && a.is_finite() {
            FpFlags::DZ
        } else if (a == 0.0 && b == 0.0) || (a.is_infinite() && b.is_infinite()) {
            FpFlags::NV
        } else {
            FpFlags::NONE
        };
        Self::round(a / b, rm, extra)
    }

    fn sqrt(self, rm: RoundingMode) -> (Self, FpFlags) {
        let a = self.to_f64();
        let extra = if self.is_signaling_nan() || a < 0.0 { FpFlags::NV } else { FpFlags::NONE };
        Self::round(a.sqrt(), rm, extra)
    }

    /// The f64 fused result can be inexact, and then either rounds twice or
    /// hides that it was inexact, so an inexact one is rounded from the
    /// exact value instead.
    fn mul_add(self, b: Self, c: Self, rm: RoundingMode) -> (Self, FpFlags) {
        let (x, y, z) = (self.to_f64(), b.to_f64(), c.to_f64());
        let signaling = self.is_signaling_nan() || b.is_signaling_nan() || c.is_signaling_nan();
        let nv = if signaling || fused_is_invalid(x, y, z) { FpFlags::NV } else { FpFlags::NONE };
        clear_host_fp_flags();
        let fused = black_box(black_box(x).mul_add(black_box(y), black_box(z)));
        let is_fused_inexact = read_host_fp_flags().contains(FpFlags::NX);
        let (bits, flags) = f64_to_f16(fused, rm);
        let is_inexact = is_fused_inexact || flags.contains(FpFlags::NX);
        if is_inexact && x.is_finite() && y.is_finite() && z.is_finite() {
            let exact = exact::mul_add(Exact::of_f64(x), Exact::of_f64(y), Exact::of_f64(z));
            let (bits, flags) = exact::round(exact, FpFormat::Half, rm);
            return (Self(bits as u16), flags | nv);
        }
        (Self(bits), flags | nv)
    }

    fn min(self, other: Self) -> Self {
        Self::of_widened(fmin_f32(f16_to_f32(self.0), f16_to_f32(other.0)))
    }

    fn max(self, other: Self) -> Self {
        Self::of_widened(fmax_f32(f16_to_f32(self.0), f16_to_f32(other.0)))
    }

    /// An integer f16 can hold is exact in f64, and a larger one overflows
    /// f16 whatever the f64 step rounded it to.
    fn from_int(value: u64, kind: (IntWidth, Signedness), rm: RoundingMode) -> (Self, FpFlags) {
        let widened = match kind {
            (IntWidth::Word, Signedness::Signed) => f64::from(value as i32),
            (IntWidth::Word, Signedness::Unsigned) => f64::from(value as u32),
            (IntWidth::Double, Signedness::Signed) => value.cast_signed() as f64,
            (IntWidth::Double, Signedness::Unsigned) => u64_to_f64(value),
        };
        Self::round(widened, rm, FpFlags::NONE)
    }

    fn from_f64_rounded(value: f64, rm: RoundingMode) -> (Self, FpFlags) {
        Self::round(value, rm, FpFlags::NONE)
    }
}

/// `value` as an f64, converted on the host. LLVM's u64-to-f64 fallback,
/// used when the host lacks CVTUSI2SD, gives -0 for zero under round-down,
/// so zero is explicit; `black_box` keeps LLVM, which assumes
/// round-to-nearest, from folding the branch away.
const fn u64_to_f64(value: u64) -> f64 {
    if value == 0 { 0.0 } else { black_box(value) as f64 }
}

/// Implements [`Float`] for a host float, whose operations run on the host
/// FPU, with RMM corrected from the exact value.
macro_rules! host_float {
    ($float:ident, $bits:ident, $format:expr, $canonical:expr, $on_host:ident, $exact:ident,
     $is_snan:ident, $min:ident, $max:ident) => {
        impl Float for $float {
            const FORMAT: FpFormat = $format;
            const BITS: u32 = $bits::BITS;
            const CANONICAL_NAN: Self = $float::from_bits($canonical);

            fn from_raw_bits(bits: u64) -> Self {
                $float::from_bits(bits as $bits)
            }

            fn raw_bits(self) -> u64 {
                u64::from(self.to_bits())
            }

            fn from_f64_exact(value: f64) -> Self {
                value as $float
            }

            fn is_nan(self) -> bool {
                $float::is_nan(self)
            }

            fn is_signaling_nan(self) -> bool {
                $is_snan(self)
            }

            fn add(self, other: Self, rm: RoundingMode) -> (Self, FpFlags) {
                $on_host(
                    rm,
                    || black_box(self) + black_box(other),
                    || exact::add(Exact::$exact(self), Exact::$exact(other)),
                )
            }

            fn sub(self, other: Self, rm: RoundingMode) -> (Self, FpFlags) {
                $on_host(
                    rm,
                    || black_box(self) - black_box(other),
                    || exact::sub(Exact::$exact(self), Exact::$exact(other)),
                )
            }

            fn mul(self, other: Self, rm: RoundingMode) -> (Self, FpFlags) {
                $on_host(
                    rm,
                    || black_box(self) * black_box(other),
                    || exact::mul(Exact::$exact(self), Exact::$exact(other)),
                )
            }

            fn div(self, other: Self, rm: RoundingMode) -> (Self, FpFlags) {
                $on_host(
                    rm,
                    || black_box(self) / black_box(other),
                    || exact::div(Exact::$exact(self), Exact::$exact(other)),
                )
            }

            fn sqrt(self, rm: RoundingMode) -> (Self, FpFlags) {
                $on_host(rm, || black_box(self).sqrt(), || exact::sqrt(Exact::$exact(self)))
            }

            /// The host signals no invalid operation for 0 × ∞ with a quiet-NaN
            /// addend, which RISC-V counts as one.
            fn mul_add(self, b: Self, c: Self, rm: RoundingMode) -> (Self, FpFlags) {
                let (result, flags) = $on_host(
                    rm,
                    || black_box(self).mul_add(black_box(b), black_box(c)),
                    || exact::mul_add(Exact::$exact(self), Exact::$exact(b), Exact::$exact(c)),
                );
                let invalid = fused_is_invalid(self.to_f64(), b.to_f64(), c.to_f64());
                (result, if invalid { flags | FpFlags::NV } else { flags })
            }

            fn min(self, other: Self) -> Self {
                $min(self, other)
            }

            fn max(self, other: Self) -> Self {
                $max(self, other)
            }

            fn from_f64_rounded(value: f64, rm: RoundingMode) -> (Self, FpFlags) {
                $on_host(rm, || black_box(value) as $float, || Exact::of_f64(value))
            }

            host_float_from_int!($float, $on_host);
        }
    };
}

/// [`Float::from_int`] and [`Float::to_f64`] for a host float.
macro_rules! host_float_from_int {
    (f32, $on_host:ident) => {
        fn to_f64(self) -> f64 {
            f64::from(self)
        }

        fn from_int(value: u64, kind: (IntWidth, Signedness), rm: RoundingMode) -> (Self, FpFlags) {
            match kind {
                (IntWidth::Word, Signedness::Signed) => {
                    let value = value as i32;
                    $on_host(rm, || black_box(value) as f32, || Exact::of_integer(value.into()))
                }
                (IntWidth::Word, Signedness::Unsigned) => {
                    let value = value as u32;
                    $on_host(rm, || black_box(value) as f32, || Exact::of_unsigned(value.into()))
                }
                (IntWidth::Double, Signedness::Signed) => {
                    let value = value.cast_signed();
                    $on_host(rm, || black_box(value) as f32, || Exact::of_integer(value.into()))
                }
                (IntWidth::Double, Signedness::Unsigned) => {
                    $on_host(rm, || black_box(value) as f32, || Exact::of_unsigned(value.into()))
                }
            }
        }
    };
    (f64, $on_host:ident) => {
        fn to_f64(self) -> f64 {
            self
        }

        fn from_int(value: u64, kind: (IntWidth, Signedness), rm: RoundingMode) -> (Self, FpFlags) {
            match kind {
                (IntWidth::Word, Signedness::Signed) => {
                    let value = value as i32;
                    $on_host(rm, || f64::from(black_box(value)), || Exact::of_integer(value.into()))
                }
                (IntWidth::Word, Signedness::Unsigned) => {
                    let value = value as u32;
                    $on_host(
                        rm,
                        || f64::from(black_box(value)),
                        || Exact::of_unsigned(value.into()),
                    )
                }
                (IntWidth::Double, Signedness::Signed) => {
                    let value = value.cast_signed();
                    $on_host(rm, || black_box(value) as f64, || Exact::of_integer(value.into()))
                }
                (IntWidth::Double, Signedness::Unsigned) => {
                    $on_host(rm, || u64_to_f64(value), || Exact::of_unsigned(value.into()))
                }
            }
        }
    };
}

host_float!(
    f32,
    u32,
    FpFormat::Single,
    0x7fc0_0000,
    on_host_f32,
    of_f32,
    is_snan_f32,
    fmin_f32,
    fmax_f32
);
host_float!(
    f64,
    u64,
    FpFormat::Double,
    0x7ff8_0000_0000_0000,
    on_host_f64,
    of_f64,
    is_snan_f64,
    fmin_f64,
    fmax_f64
);
