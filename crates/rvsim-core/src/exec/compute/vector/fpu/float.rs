//! The element formats the vector FP unit computes in, so that each
//! operation is written once: f16 in software, f32 and f64 on the host.

use super::estimate::{vfrec7_16, vfrec7_32, vfrec7_64, vfrsqrt7_16, vfrsqrt7_32, vfrsqrt7_64};
use crate::exec::compute::fpu::exact::{self, Exact, on_host_f32, on_host_f64};
use crate::exec::compute::fpu::half::{
    CANONICAL_NAN_F16, F16_SIGN, f16_to_f32, f64_to_f16, fused_is_invalid, is_snan_f16,
    product_is_invalid, sum_is_invalid,
};
use crate::exec::compute::fpu::nan_handling::{
    box_f32, box_f32_canon, canonicalize_f64_bits, fmax_f32, fmax_f64, fmin_f32, fmin_f64,
    is_snan_f32, is_snan_f64,
};
use crate::isa::fp::{FpFlags, FpFormat, RoundingMode};
use crate::isa::rvv::Sew;
use std::hint::black_box;
use std::ops::Neg;

/// An element format: its value as an element holds it, and its
/// operations rounded under a rounding mode with the IEEE flags they raise.
pub trait VecFloat: Copy + Neg<Output = Self> {
    /// Its format.
    const FORMAT: FpFormat;
    /// Its element width.
    const SEW: Sew;
    /// Its sign bit within the element.
    const SIGN_BIT: u64;

    /// The value the element `bits` holds.
    fn from_elem(bits: u64) -> Self;
    /// Its bits as an element, NaNs as they are.
    fn elem_bits(self) -> u64;
    /// Its bits as an element, with a NaN replaced by the canonical NaN.
    fn canonical_elem_bits(self) -> u64;
    /// Its value as an f64, which every format widens to exactly.
    fn to_f64(self) -> f64;
    /// `value`, which the format holds exactly: a narrower format's value
    /// or a NaN.
    fn from_f64_exact(value: f64) -> Self;
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
    /// The unsigned integer `value`, which fits in [`Self::SEW`] bits.
    fn from_unsigned(value: u64, rm: RoundingMode) -> (Self, FpFlags);
    /// The signed integer `value`, which fits in [`Self::SEW`] bits.
    fn from_signed(value: i64, rm: RoundingMode) -> (Self, FpFlags);
    /// `vfrsqrt7` of the element `bits`.
    fn rsqrt7(bits: u64) -> (u64, FpFlags);
    /// `vfrec7` of the element `bits`.
    fn rec7(bits: u64, rm: RoundingMode) -> (u64, FpFlags);
}

/// An f16 element, computed through f64, which holds every f16 sum,
/// difference and product exactly, and rounded to f16 in software.
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
            return Self(CANONICAL_NAN_F16);
        }
        Self(f64_to_f16(f64::from(value), RoundingMode::Rne).0)
    }
}

impl Neg for SoftF16 {
    type Output = Self;

    fn neg(self) -> Self {
        Self(self.0 ^ F16_SIGN)
    }
}

impl VecFloat for SoftF16 {
    const FORMAT: FpFormat = FpFormat::Half;
    const SEW: Sew = Sew::E16;
    const SIGN_BIT: u64 = F16_SIGN as u64;

    fn from_elem(bits: u64) -> Self {
        Self(bits as u16)
    }

    fn elem_bits(self) -> u64 {
        u64::from(self.0)
    }

    fn canonical_elem_bits(self) -> u64 {
        u64::from(self.0)
    }

    fn to_f64(self) -> f64 {
        f64::from(f16_to_f32(self.0))
    }

    fn from_f64_exact(value: f64) -> Self {
        Self(f64_to_f16(value, RoundingMode::Rne).0)
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

    /// The f64 fused result can be inexact and then rounds twice, so an
    /// inexact one is rounded from the exact value.
    fn mul_add(self, b: Self, c: Self, rm: RoundingMode) -> (Self, FpFlags) {
        let (x, y, z) = (self.to_f64(), b.to_f64(), c.to_f64());
        let signaling = self.is_signaling_nan() || b.is_signaling_nan() || c.is_signaling_nan();
        let nv = if signaling || fused_is_invalid(x, y, z) { FpFlags::NV } else { FpFlags::NONE };
        let (bits, flags) = f64_to_f16(x.mul_add(y, z), rm);
        if flags.contains(FpFlags::NX) && x.is_finite() && y.is_finite() && z.is_finite() {
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

    fn from_unsigned(value: u64, rm: RoundingMode) -> (Self, FpFlags) {
        Self::round(value as f64, rm, FpFlags::NONE)
    }

    fn from_signed(value: i64, rm: RoundingMode) -> (Self, FpFlags) {
        Self::round(value as f64, rm, FpFlags::NONE)
    }

    fn rsqrt7(bits: u64) -> (u64, FpFlags) {
        let (estimate, flags) = vfrsqrt7_16(bits as u16);
        (u64::from(estimate), flags)
    }

    fn rec7(bits: u64, rm: RoundingMode) -> (u64, FpFlags) {
        let (estimate, flags) = vfrec7_16(bits as u16, rm);
        (u64::from(estimate), flags)
    }
}

/// Implements [`VecFloat`] for a host float, whose operations run on the
/// host FPU under the rounding mode, with RMM corrected from the exact
/// value.
macro_rules! host_float {
    ($float:ident, $format:expr, $sew:expr, $on_host:ident, $exact:ident, $is_snan:ident,
     $min:ident, $max:ident, $rsqrt7:ident, $rec7:ident) => {
        impl VecFloat for $float {
            const FORMAT: FpFormat = $format;
            const SEW: Sew = $sew;
            const SIGN_BIT: u64 = 1 << ($sew.bits() - 1);

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

            fn mul_add(self, b: Self, c: Self, rm: RoundingMode) -> (Self, FpFlags) {
                $on_host(
                    rm,
                    || black_box(self).mul_add(black_box(b), black_box(c)),
                    || exact::mul_add(Exact::$exact(self), Exact::$exact(b), Exact::$exact(c)),
                )
            }

            fn min(self, other: Self) -> Self {
                $min(self, other)
            }

            fn max(self, other: Self) -> Self {
                $max(self, other)
            }

            fn is_signaling_nan(self) -> bool {
                $is_snan(self)
            }

            fn rsqrt7(bits: u64) -> (u64, FpFlags) {
                $rsqrt7(bits as _)
            }

            fn rec7(bits: u64, rm: RoundingMode) -> (u64, FpFlags) {
                $rec7(bits as _, rm)
            }

            host_float_conversions!($float, $on_host);
        }
    };
}

/// The element and integer conversions of a host float: the parts of
/// [`VecFloat`] that differ between f32 and f64 beyond naming.
macro_rules! host_float_conversions {
    (f32, $on_host:ident) => {
        fn from_elem(bits: u64) -> Self {
            f32::from_bits(bits as u32)
        }

        fn elem_bits(self) -> u64 {
            box_f32(self)
        }

        fn canonical_elem_bits(self) -> u64 {
            box_f32_canon(self)
        }

        fn to_f64(self) -> f64 {
            f64::from(self)
        }

        fn from_f64_exact(value: f64) -> Self {
            value as f32
        }

        fn from_unsigned(value: u64, rm: RoundingMode) -> (Self, FpFlags) {
            let value = value as u32;
            $on_host(rm, || black_box(value) as f32, || Exact::of_unsigned(u128::from(value)))
        }

        fn from_signed(value: i64, rm: RoundingMode) -> (Self, FpFlags) {
            let value = value as i32;
            $on_host(rm, || black_box(value) as f32, || Exact::of_integer(i128::from(value)))
        }
    };
    (f64, $on_host:ident) => {
        fn from_elem(bits: u64) -> Self {
            f64::from_bits(bits)
        }

        fn elem_bits(self) -> u64 {
            self.to_bits()
        }

        fn canonical_elem_bits(self) -> u64 {
            canonicalize_f64_bits(self)
        }

        fn to_f64(self) -> f64 {
            self
        }

        fn from_f64_exact(value: f64) -> Self {
            value
        }

        // Workaround for an LLVM codegen quirk: `0u64 as f64` under host
        // FE_DOWNWARD/FE_UPWARD produces a signed zero with the wrong sign
        // (the software fallback used when CVTUSI2SD_q is unavailable
        // doesn't short-circuit the zero case). Treat zero explicitly.
        fn from_unsigned(value: u64, rm: RoundingMode) -> (Self, FpFlags) {
            $on_host(
                rm,
                || if value == 0 { 0.0 } else { black_box(value) as f64 },
                || Exact::of_unsigned(u128::from(value)),
            )
        }

        fn from_signed(value: i64, rm: RoundingMode) -> (Self, FpFlags) {
            $on_host(rm, || black_box(value) as f64, || Exact::of_integer(i128::from(value)))
        }
    };
}

host_float!(
    f32,
    FpFormat::Single,
    Sew::E32,
    on_host_f32,
    of_f32,
    is_snan_f32,
    fmin_f32,
    fmax_f32,
    vfrsqrt7_32,
    vfrec7_32
);
host_float!(
    f64,
    FpFormat::Double,
    Sew::E64,
    on_host_f64,
    of_f64,
    is_snan_f64,
    fmin_f64,
    fmax_f64,
    vfrsqrt7_64,
    vfrec7_64
);
