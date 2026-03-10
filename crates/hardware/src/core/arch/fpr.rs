//! RISC-V Floating-Point Register File.
//!
//! This module implements the Floating-Point Register (FPR) file for the RISC-V architecture.
//! It performs the following:
//! 1. **Storage:** Maintains 32 floating-point registers (`f0`-`f31`).
//! 2. **Type Conversion:** Handles conversion between 64-bit IEEE 754 raw bits and internal representation.
//! 3. **Access Control:** Provides methods for reading and writing double-precision values.

use crate::common::RegIdx;

/// Floating-Point Register file.
///
/// Contains 32 floating-point registers used for arithmetic operations. Registers
/// are stored as 64-bit double-precision values.
#[derive(Debug)]
pub struct Fpr {
    fregs: [f64; 32],
}

impl Default for Fpr {
    fn default() -> Self {
        Self::new()
    }
}

impl Fpr {
    /// Creates a new floating-point register file with all registers initialized to zero.
    ///
    /// # Returns
    ///
    /// A new `Fpr` instance with all registers set to `0.0`.
    pub const fn new() -> Self {
        Self { fregs: [0.0; 32] }
    }

    /// Reads a floating-point register value as raw bits.
    ///
    /// # Arguments
    ///
    /// * `idx` - Floating-point register index (f0-f31).
    ///
    /// # Returns
    ///
    /// The 64-bit IEEE 754 representation of the floating-point value.
    pub const fn read(&self, idx: RegIdx) -> u64 {
        self.fregs[idx.as_usize()].to_bits()
    }

    /// Writes a floating-point register value from raw bits.
    ///
    /// # Arguments
    ///
    /// * `idx` - Floating-point register index (f0-f31).
    /// * `val` - The 64-bit IEEE 754 representation of the floating-point value.
    pub const fn write(&mut self, idx: RegIdx, val: u64) {
        self.fregs[idx.as_usize()] = f64::from_bits(val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fpr_default_and_new() {
        let fpr1 = Fpr::default();
        let fpr2 = Fpr::new();
        for i in 0..32u8 {
            assert_eq!(fpr1.read(RegIdx::new(i)), 0);
            assert_eq!(fpr2.read(RegIdx::new(i)), 0);
        }
    }

    #[test]
    fn test_fpr_read_write() {
        let mut fpr = Fpr::new();
        let val = core::f64::consts::PI.to_bits();
        fpr.write(RegIdx::new(5), val);
        assert_eq!(fpr.read(RegIdx::new(5)), val);
        assert_eq!(fpr.read(RegIdx::new(0)), 0); // other registers unchanged
    }
}
