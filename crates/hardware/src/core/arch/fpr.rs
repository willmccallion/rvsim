//! RISC-V Floating-Point Register File.
//!
//! This module implements the Floating-Point Register (FPR) file for the RISC-V architecture.
//! It performs the following:
//! 1. **Storage:** Maintains 32 floating-point registers (`f0`-`f31`).
//! 2. **Type Conversion:** Handles conversion between 64-bit IEEE 754 raw bits and internal representation.
//! 3. **Access Control:** Provides methods for reading and writing double-precision values.

/// Floating-Point Register file.
///
/// Contains 32 floating-point registers used for arithmetic operations. Registers
/// are stored as 64-bit double-precision values.
pub struct Fpr {
    fregs: [f64; 32],
}

impl Fpr {
    /// Creates a new floating-point register file with all registers initialized to zero.
    ///
    /// # Returns
    ///
    /// A new `Fpr` instance with all registers set to `0.0`.
    pub fn new() -> Self {
        Self { fregs: [0.0; 32] }
    }

    /// Reads a floating-point register value as raw bits.
    ///
    /// # Arguments
    ///
    /// * `idx` - Register index (0-31).
    ///
    /// # Returns
    ///
    /// The 64-bit IEEE 754 representation of the floating-point value.
    pub fn read(&self, idx: usize) -> u64 {
        self.fregs[idx].to_bits()
    }

    /// Writes a floating-point register value from raw bits.
    ///
    /// # Arguments
    ///
    /// * `idx` - Register index (0-31).
    /// * `val` - The 64-bit IEEE 754 representation of the floating-point value.
    pub fn write(&mut self, idx: usize, val: u64) {
        self.fregs[idx] = f64::from_bits(val);
    }
}
