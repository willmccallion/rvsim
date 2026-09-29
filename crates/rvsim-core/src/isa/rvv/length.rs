//! Vector lengths: `VLEN`, `VLMAX`, and `vl`.

use crate::isa::rvv::{Sew, Vlmul};

/// Vector length (vl CSR value). NOT interchangeable with `Vlmax` or element count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Vl(u64);

impl Vl {
    /// Create a new vector length value.
    #[inline(always)]
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Returns the value as `u64`.
    #[inline(always)]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Returns the value as `usize`.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Returns true if the vector length is zero.
    #[inline(always)]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

/// VLMAX value (maximum vector length for current vtype). Derived, never directly set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Vlmax(usize);

impl Vlmax {
    /// Compute VLMAX = (VLEN / SEW) * LMUL.
    pub const fn compute(vlen: Vlen, sew: Sew, lmul: Vlmul) -> Self {
        let vlen_bits = vlen.bits();
        let sew_bits = sew.bits();
        let (num, den) = lmul.as_fraction();
        // VLMAX = (VLEN / SEW) * (LMUL_num / LMUL_den)
        Self((vlen_bits / sew_bits) * num / den)
    }

    /// Returns the value as `usize`.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }

    /// Returns the value as `u64`.
    #[inline(always)]
    pub const fn as_u64(self) -> u64 {
        self.0 as u64
    }
}

/// VLEN (vector register width in bits). Immutable per-core configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Vlen(usize);

impl Vlen {
    /// Creates a `Vlen` from a raw value.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the value is not a power of 2 in range [128, 2048].
    pub const fn new(val: usize) -> Result<Self, &'static str> {
        if !val.is_power_of_two() || val < 128 || val > 2048 {
            return Err("VLEN must be power of 2 in range [128, 2048]");
        }
        Ok(Self(val))
    }

    /// Creates a `Vlen` without validation. For use in const contexts where
    /// the value is known valid.
    ///
    /// # Safety (logical)
    ///
    /// Caller must ensure val is a power of 2 in [128, 2048].
    pub const fn new_unchecked(val: usize) -> Self {
        Self(val)
    }

    /// Width in bits.
    #[inline(always)]
    pub const fn bits(self) -> usize {
        self.0
    }

    /// Width in bytes.
    #[inline(always)]
    pub const fn bytes(self) -> usize {
        self.0 / 8
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_vlmax_computation() {
        let vlen = Vlen::new_unchecked(128);
        assert_eq!(Vlmax::compute(vlen, Sew::E8, Vlmul::M1).as_usize(), 16);
        assert_eq!(Vlmax::compute(vlen, Sew::E32, Vlmul::M1).as_usize(), 4);
        assert_eq!(Vlmax::compute(vlen, Sew::E8, Vlmul::M8).as_usize(), 128);
        assert_eq!(Vlmax::compute(vlen, Sew::E64, Vlmul::Mf8).as_usize(), 0);

        let vlen256 = Vlen::new_unchecked(256);
        assert_eq!(Vlmax::compute(vlen256, Sew::E32, Vlmul::M2).as_usize(), 16);
    }

    #[test]
    fn test_vlen_validation() {
        assert!(Vlen::new(128).is_ok());
        assert!(Vlen::new(256).is_ok());
        assert!(Vlen::new(2048).is_ok());
        assert!(Vlen::new(64).is_err());
        assert!(Vlen::new(100).is_err());
        assert!(Vlen::new(4096).is_err());
    }
}
