//! The architectural register files: GPRs, FPRs and VPRs.

pub mod fpr;

pub mod gpr;

pub mod vpr;

use crate::arch::regs::fpr::Fpr;
use crate::arch::regs::gpr::Gpr;
use crate::arch::regs::vpr::Vpr;
use crate::isa::reg::RegIdx;
use crate::isa::rvv::Vlen;

/// Unified register file containing general-purpose, floating-point, and vector registers.
///
/// This structure provides a single interface for accessing all processor registers,
/// abstracting the underlying GPR, FPR, and VPR implementations.
#[derive(Debug)]
pub struct RegisterFile {
    gpr: Gpr,
    fpr: Fpr,
    vpr: Vpr,
}

impl Default for RegisterFile {
    /// Zeroed registers with V's minimum VLEN.
    fn default() -> Self {
        Self::new(Vlen::default())
    }
}

impl RegisterFile {
    /// Creates a register file with every register zero and vector
    /// registers `vlen` bits wide.
    pub fn new(vlen: Vlen) -> Self {
        Self { gpr: Gpr::new(), fpr: Fpr::new(), vpr: Vpr::new(vlen) }
    }

    /// The vector register file.
    pub const fn vpr(&self) -> &Vpr {
        &self.vpr
    }

    /// The vector register file, to write.
    pub const fn vpr_mut(&mut self) -> &mut Vpr {
        &mut self.vpr
    }

    /// Reads a value from a general-purpose register. Register `x0` always returns 0.
    pub const fn read(&self, idx: RegIdx) -> u64 {
        self.gpr.read(idx)
    }

    /// Writes a value to a general-purpose register. Writes to `x0` are ignored.
    pub const fn write(&mut self, idx: RegIdx, val: u64) {
        self.gpr.write(idx, val);
    }

    /// Reads a value from a floating-point register.
    pub const fn read_f(&self, idx: RegIdx) -> u64 {
        self.fpr.read(idx)
    }

    /// Writes a value to a floating-point register.
    pub const fn write_f(&mut self, idx: RegIdx, val: u64) {
        self.fpr.write(idx, val);
    }

    /// The general-purpose registers, to display.
    pub const fn gpr(&self) -> &Gpr {
        &self.gpr
    }
}
