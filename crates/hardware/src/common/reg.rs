//! Unified Register File.
//!
//! This module provides the `RegisterFile` struct, which acts as a unified interface for
//! accessing both General Purpose Registers (GPRs) and Floating-Point Registers (FPRs).
//! It provides:
//! 1. **Unified Storage:** Combined storage for all RISC-V architectural registers.
//! 2. **Abstraction:** A single set of methods for reading and writing register values.
//! 3. **Observability:** Debugging utilities for dumping register state during simulation.

use crate::common::RegIdx;
use crate::core::arch::fpr::Fpr;
use crate::core::arch::gpr::Gpr;
use crate::core::arch::vpr::Vpr;
use crate::core::units::vpu::types::Vlen;

/// Unified register file containing general-purpose, floating-point, and vector registers.
///
/// This structure provides a single interface for accessing all processor registers,
/// abstracting the underlying GPR, FPR, and VPR implementations.
#[derive(Debug)]
pub struct RegisterFile {
    gpr: Gpr,
    fpr: Fpr,
    vpr: Option<Vpr>,
}

impl Default for RegisterFile {
    fn default() -> Self {
        Self::new()
    }
}

impl RegisterFile {
    /// Creates a new register file with all registers initialized to zero.
    /// Vector registers are not allocated (use `init_vpr` to enable).
    pub const fn new() -> Self {
        Self { gpr: Gpr::new(), fpr: Fpr::new(), vpr: None }
    }

    /// Initializes the vector register file with the given VLEN.
    pub fn init_vpr(&mut self, vlen: Vlen) {
        self.vpr = Some(Vpr::new(vlen));
    }

    /// Returns a reference to the vector register file.
    ///
    /// # Panics
    ///
    /// Panics if the VPR has not been initialized.
    #[allow(clippy::option_if_let_else)]
    pub fn vpr(&self) -> &Vpr {
        match &self.vpr {
            Some(v) => v,
            None => panic!("VPR not initialized"),
        }
    }

    /// Returns a mutable reference to the vector register file.
    ///
    /// # Panics
    ///
    /// Panics if the VPR has not been initialized.
    #[allow(clippy::option_if_let_else)]
    pub fn vpr_mut(&mut self) -> &mut Vpr {
        match &mut self.vpr {
            Some(v) => v,
            None => panic!("VPR not initialized"),
        }
    }

    /// Returns true if the vector register file is initialized.
    pub const fn has_vpr(&self) -> bool {
        self.vpr.is_some()
    }

    /// Reads a value from a general-purpose register.
    ///
    /// # Arguments
    ///
    /// * `idx` - Register index (x0-x31). Register `x0` always returns 0.
    ///
    /// # Returns
    ///
    /// The 64-bit value stored in the specified register.
    pub const fn read(&self, idx: RegIdx) -> u64 {
        self.gpr.read(idx)
    }

    /// Writes a value to a general-purpose register.
    ///
    /// # Arguments
    ///
    /// * `idx` - Register index (x0-x31). Writes to `x0` are ignored.
    /// * `val` - The 64-bit value to write.
    pub const fn write(&mut self, idx: RegIdx, val: u64) {
        self.gpr.write(idx, val);
    }

    /// Reads a value from a floating-point register.
    ///
    /// # Arguments
    ///
    /// * `idx` - Floating-point register index (f0-f31).
    ///
    /// # Returns
    ///
    /// The 64-bit value stored in the specified floating-point register.
    pub const fn read_f(&self, idx: RegIdx) -> u64 {
        self.fpr.read(idx)
    }

    /// Writes a value to a floating-point register.
    ///
    /// # Arguments
    ///
    /// * `idx` - Floating-point register index (f0-f31).
    /// * `val` - The 64-bit value to write.
    pub const fn write_f(&mut self, idx: RegIdx, val: u64) {
        self.fpr.write(idx, val);
    }

    /// Dumps the contents of all general-purpose registers to stderr.
    ///
    /// Useful for debugging and tracing register state during simulation.
    pub fn dump(&self) {
        self.gpr.dump();
    }
}
