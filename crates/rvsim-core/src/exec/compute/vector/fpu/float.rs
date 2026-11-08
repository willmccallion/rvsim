//! What the vector unit adds to a [`Float`] format: its element width and
//! the reciprocal estimates.

use super::estimate::{vfrec7_16, vfrec7_32, vfrec7_64, vfrsqrt7_16, vfrsqrt7_32, vfrsqrt7_64};
use crate::exec::compute::fpu::float::{Float, SoftF16};
use crate::isa::fp::{FpFlags, RoundingMode};
use crate::isa::op::IntWidth;
use crate::isa::rvv::Sew;

/// A [`Float`] format as a vector element.
pub trait VecFloat: Float {
    /// Its element width.
    const SEW: Sew;
    /// The width of the integer a same-width conversion reads or writes,
    /// which holds every SEW-bit integer.
    const INT_WIDTH: IntWidth;

    /// `vfrsqrt7` of the element `bits`.
    fn rsqrt7(bits: u64) -> (u64, FpFlags);
    /// `vfrec7` of the element `bits`.
    fn rec7(bits: u64, rm: RoundingMode) -> (u64, FpFlags);
}

impl VecFloat for SoftF16 {
    const SEW: Sew = Sew::E16;
    const INT_WIDTH: IntWidth = IntWidth::Word;

    fn rsqrt7(bits: u64) -> (u64, FpFlags) {
        let (estimate, flags) = vfrsqrt7_16(bits as u16);
        (u64::from(estimate), flags)
    }

    fn rec7(bits: u64, rm: RoundingMode) -> (u64, FpFlags) {
        let (estimate, flags) = vfrec7_16(bits as u16, rm);
        (u64::from(estimate), flags)
    }
}

impl VecFloat for f32 {
    const SEW: Sew = Sew::E32;
    const INT_WIDTH: IntWidth = IntWidth::Word;

    fn rsqrt7(bits: u64) -> (u64, FpFlags) {
        vfrsqrt7_32(bits as u32)
    }

    fn rec7(bits: u64, rm: RoundingMode) -> (u64, FpFlags) {
        vfrec7_32(bits as u32, rm)
    }
}

impl VecFloat for f64 {
    const SEW: Sew = Sew::E64;
    const INT_WIDTH: IntWidth = IntWidth::Double;

    fn rsqrt7(bits: u64) -> (u64, FpFlags) {
        vfrsqrt7_64(bits)
    }

    fn rec7(bits: u64, rm: RoundingMode) -> (u64, FpFlags) {
        vfrec7_64(bits, rm)
    }
}
