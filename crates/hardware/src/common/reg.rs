//! Unified Register File.
//!
//! This module provides the `RegisterFile` struct, which acts as a unified interface for
//! accessing both General Purpose Registers (GPRs) and Floating-Point Registers (FPRs).
//! It provides:
//! 1. **Unified Storage:** Combined storage for all RISC-V architectural registers.
//! 2. **Abstraction:** A single set of methods for reading and writing register values.
//! 3. **Observability:** Debugging utilities for dumping register state during simulation.

use crate::core::arch::fpr::Fpr;
use crate::core::arch::gpr::Gpr;

/// Unified register file containing both general-purpose and floating-point registers.
///
/// This structure provides a single interface for accessing all processor registers,
/// abstracting the underlying GPR and FPR implementations.
pub struct RegisterFile {
    gpr: Gpr,
    fpr: Fpr,
}

impl RegisterFile {
    /// Creates a new register file with all registers initialized to zero.
    ///
    /// # Returns
    ///
    /// A new `RegisterFile` instance with initialized GPR and FPR components.
    pub fn new() -> Self {
        Self {
            gpr: Gpr::new(),
            fpr: Fpr::new(),
        }
    }

    /// Reads a value from a general-purpose register.
    ///
    /// # Arguments
    ///
    /// * `idx` - Register index (0-31). Register `x0` always returns 0.
    ///
    /// # Returns
    ///
    /// The 64-bit value stored in the specified register.
    pub fn read(&self, idx: usize) -> u64 {
        self.gpr.read(idx)
    }

    /// Writes a value to a general-purpose register.
    ///
    /// # Arguments
    ///
    /// * `idx` - Register index (0-31). Writes to `x0` are ignored.
    /// * `val` - The 64-bit value to write.
    pub fn write(&mut self, idx: usize, val: u64) {
        self.gpr.write(idx, val);
    }

    /// Reads a value from a floating-point register.
    ///
    /// # Arguments
    ///
    /// * `idx` - Floating-point register index (0-31).
    ///
    /// # Returns
    ///
    /// The 64-bit value stored in the specified floating-point register.
    pub fn read_f(&self, idx: usize) -> u64 {
        self.fpr.read(idx)
    }

    /// Writes a value to a floating-point register.
    ///
    /// # Arguments
    ///
    /// * `idx` - Floating-point register index (0-31).
    /// * `val` - The 64-bit value to write.
    pub fn write_f(&mut self, idx: usize, val: u64) {
        self.fpr.write(idx, val);
    }

    /// Dumps the contents of all general-purpose registers to stderr.
    ///
    /// Useful for debugging and tracing register state during simulation.
    pub fn dump(&self) {
        self.gpr.dump();
    }
}
