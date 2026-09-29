//! A RISC-V hardware thread.
//!
//! `Hart` owns the per-thread architectural state: registers, CSRs, program
//! counter, privilege mode, MMU, PMP, and load reservation. On a non-SMT core
//! there is exactly one `Hart`; with SMT, sibling threads share the parent
//! [`Core`](crate::uarch::Core)'s pipeline and L1 caches but each retains its own
//! `Hart`.
//!
//! Constructed with a [`HartId`] that the `mhartid` CSR reports.

use crate::arch::csr::Csrs;
use crate::arch::pmp::Pmp;
use crate::arch::regs::RegisterFile;
use crate::common::HartId;
use crate::isa::privileged::PrivilegeMode;

/// Per-thread RISC-V architectural state.
#[derive(Debug)]
pub struct Hart {
    /// Globally unique hardware-thread identifier; reported by `mhartid`.
    pub hart_id: HartId,
    /// General Purpose and Floating Point Registers.
    pub regs: RegisterFile,
    /// The architectural program counter: the next instruction to retire.
    /// Fetch runs ahead of it on the pipeline's own fetch PC.
    pub pc: u64,
    /// Control and Status Registers.
    pub csrs: Csrs,
    /// Current Privilege Mode (M, S, U).
    pub privilege: PrivilegeMode,
    /// Physical Memory Protection unit.
    pub pmp: Pmp,
    /// True when the hart has executed `WFI` and is waiting for an interrupt.
    pub wfi_waiting: bool,
    /// Software-written SEIP bit. The `mip` SEIP bit is the OR of this and
    /// the PLIC hardware signal, so the software component is tracked here.
    pub sw_seip: bool,
    /// Instructions this hart has retired; backs `instret` / `minstret`.
    pub instructions_retired: u64,
}

/// Initial values for constructing a [`Hart`].
///
/// Keeps the [`Hart::new`] signature short while making the inputs explicit at
/// the call site.
#[derive(Debug)]
pub struct HartInit {
    /// Globally unique hardware-thread identifier.
    pub hart_id: HartId,
    /// Initial register file (with stack pointer set if running bare-metal).
    pub regs: RegisterFile,
    /// Initial program counter.
    pub pc: u64,
    /// Initial CSR state.
    pub csrs: Csrs,
    /// Initial privilege mode.
    pub privilege: PrivilegeMode,
    /// Physical memory protection unit.
    pub pmp: Pmp,
}

impl Hart {
    /// Creates a new `Hart` from its initial configuration.
    pub fn new(init: HartInit) -> Self {
        Self {
            hart_id: init.hart_id,
            regs: init.regs,
            pc: init.pc,
            csrs: init.csrs,
            privilege: init.privilege,
            pmp: init.pmp,
            wfi_waiting: false,
            sw_seip: false,
            instructions_retired: 0,
        }
    }
}

impl Hart {
    /// Dumps the current hart state (PC and registers) to stdout.
    pub fn dump_state(&self) {
        println!("PC = {:#018x}", self.pc);
        self.regs.dump();
    }
}
