//! Trap and Translation Result definitions.
//!
//! This module defines the error handling and trap mechanisms for the simulator. It provides:
//! 1. **Trap Representation:** Encompassing all synchronous exceptions and asynchronous interrupts.
//! 2. **Translation Results:** Reporting the outcome of virtual-to-physical address translation.
//! 3. **Error Handling:** Integrating with standard Rust error traits for system-level reporting.

use std::fmt;

use super::addr::PhysAddr;

/// RISC-V trap types representing exceptions and interrupts.
///
/// Traps cause the processor to transfer control to a predefined trap handler.
/// This enum covers all standard traps defined in the RISC-V Privileged Specification.
#[derive(Clone, Debug, PartialEq)]
pub enum Trap {
    /// Instruction address misaligned exception.
    ///
    /// Raised when the program counter is not aligned to the instruction size.
    /// The associated value is the misaligned address.
    InstructionAddressMisaligned(u64),

    /// Instruction access fault exception.
    ///
    /// Raised when an instruction fetch violates memory protection or accesses
    /// invalid memory. The associated value is the faulting address.
    InstructionAccessFault(u64),

    /// Illegal instruction exception.
    ///
    /// Raised when an instruction encoding is invalid or not implemented.
    /// The associated value is the instruction encoding.
    IllegalInstruction(u32),

    /// Breakpoint exception.
    ///
    /// Raised when a breakpoint instruction is executed or a hardware breakpoint
    /// is hit. The associated value is the program counter.
    Breakpoint(u64),

    /// Load address misaligned exception.
    ///
    /// Raised when a load instruction accesses a misaligned address.
    /// The associated value is the misaligned address.
    LoadAddressMisaligned(u64),

    /// Load access fault exception.
    ///
    /// Raised when a load instruction violates memory protection or accesses
    /// invalid memory. The associated value is the faulting address.
    LoadAccessFault(u64),

    /// Store address misaligned exception.
    ///
    /// Raised when a store instruction accesses a misaligned address.
    /// The associated value is the misaligned address.
    StoreAddressMisaligned(u64),

    /// Store access fault exception.
    ///
    /// Raised when a store instruction violates memory protection or accesses
    /// invalid memory. The associated value is the faulting address.
    StoreAccessFault(u64),

    /// Environment call from user mode.
    ///
    /// Raised when an `ECALL` instruction is executed in user mode.
    EnvironmentCallFromUMode,

    /// Environment call from supervisor mode.
    ///
    /// Raised when an `ECALL` instruction is executed in supervisor mode.
    EnvironmentCallFromSMode,

    /// Environment call from machine mode.
    ///
    /// Raised when an `ECALL` instruction is executed in machine mode.
    EnvironmentCallFromMMode,

    /// Instruction page fault exception.
    ///
    /// Raised when an instruction fetch causes a page fault.
    /// The associated value is the faulting virtual address.
    InstructionPageFault(u64),

    /// Load page fault exception.
    ///
    /// Raised when a load instruction causes a page fault.
    /// The associated value is the faulting virtual address.
    LoadPageFault(u64),

    /// Store page fault exception.
    ///
    /// Raised when a store instruction causes a page fault.
    /// The associated value is the faulting virtual address.
    StorePageFault(u64),

    /// User software interrupt.
    ///
    /// Software interrupt intended for user mode.
    UserSoftwareInterrupt,

    /// Supervisor software interrupt.
    ///
    /// Software interrupt intended for supervisor mode.
    SupervisorSoftwareInterrupt,

    /// Machine software interrupt.
    ///
    /// Software interrupt intended for machine mode.
    MachineSoftwareInterrupt,

    /// Machine timer interrupt.
    ///
    /// Timer interrupt intended for machine mode.
    MachineTimerInterrupt,

    /// Supervisor timer interrupt.
    ///
    /// Timer interrupt intended for supervisor mode.
    SupervisorTimerInterrupt,

    /// Machine external interrupt.
    ///
    /// External interrupt intended for machine mode.
    MachineExternalInterrupt,

    /// Supervisor external interrupt.
    ///
    /// External interrupt intended for supervisor mode.
    SupervisorExternalInterrupt,

    /// User external interrupt.
    ///
    /// External interrupt intended for user mode.
    UserExternalInterrupt,

    /// Requested trap for debugging or simulation purposes.
    ///
    /// The associated value is a trap code for identification.
    RequestedTrap(u64),

    /// Double fault exception.
    ///
    /// Raised when a fault occurs while handling another fault.
    /// The associated value is the faulting address.
    DoubleFault(u64),
}

impl fmt::Display for Trap {
    /// Formats the trap for display.
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to.
    ///
    /// # Returns
    ///
    /// A formatting result indicating success or failure.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trap::InstructionAddressMisaligned(addr) => {
                write!(f, "InstructionAddressMisaligned({:#x})", addr)
            }
            Trap::InstructionAccessFault(addr) => {
                write!(f, "InstructionAccessFault({:#x})", addr)
            }
            Trap::IllegalInstruction(inst) => write!(f, "IllegalInstruction({:#x})", inst),
            Trap::Breakpoint(pc) => write!(f, "Breakpoint({:#x})", pc),
            Trap::LoadAddressMisaligned(addr) => write!(f, "LoadAddressMisaligned({:#x})", addr),
            Trap::LoadAccessFault(addr) => write!(f, "LoadAccessFault({:#x})", addr),
            Trap::StoreAddressMisaligned(addr) => {
                write!(f, "StoreAddressMisaligned({:#x})", addr)
            }
            Trap::StoreAccessFault(addr) => write!(f, "StoreAccessFault({:#x})", addr),
            Trap::EnvironmentCallFromUMode => write!(f, "EnvironmentCallFromUMode"),
            Trap::EnvironmentCallFromSMode => write!(f, "EnvironmentCallFromSMode"),
            Trap::EnvironmentCallFromMMode => write!(f, "EnvironmentCallFromMMode"),
            Trap::InstructionPageFault(addr) => write!(f, "InstructionPageFault({:#x})", addr),
            Trap::LoadPageFault(addr) => write!(f, "LoadPageFault({:#x})", addr),
            Trap::StorePageFault(addr) => write!(f, "StorePageFault({:#x})", addr),
            Trap::UserSoftwareInterrupt => write!(f, "UserSoftwareInterrupt"),
            Trap::SupervisorSoftwareInterrupt => write!(f, "SupervisorSoftwareInterrupt"),
            Trap::MachineSoftwareInterrupt => write!(f, "MachineSoftwareInterrupt"),
            Trap::MachineTimerInterrupt => write!(f, "MachineTimerInterrupt"),
            Trap::SupervisorTimerInterrupt => write!(f, "SupervisorTimerInterrupt"),
            Trap::MachineExternalInterrupt => write!(f, "MachineExternalInterrupt"),
            Trap::SupervisorExternalInterrupt => write!(f, "SupervisorExternalInterrupt"),
            Trap::UserExternalInterrupt => write!(f, "UserExternalInterrupt"),
            Trap::RequestedTrap(code) => write!(f, "RequestedTrap({})", code),
            Trap::DoubleFault(addr) => write!(f, "DoubleFault({:#x})", addr),
        }
    }
}

impl std::error::Error for Trap {}

/// Result of a virtual-to-physical address translation operation.
///
/// This structure encapsulates the outcome of an MMU walk, including performance
/// metrics and any faults that may have occurred.
pub struct TranslationResult {
    /// The translated physical address, or zero if translation failed.
    pub paddr: PhysAddr,
    /// Number of cycles consumed by the translation operation.
    pub cycles: u64,
    /// Trap that occurred during translation, if any.
    pub trap: Option<Trap>,
}

impl TranslationResult {
    /// Creates a successful translation result.
    ///
    /// # Arguments
    ///
    /// * `paddr` - The successfully translated physical address.
    /// * `cycles` - Number of cycles consumed by the translation.
    ///
    /// # Returns
    ///
    /// A `TranslationResult` indicating successful translation.
    #[inline]
    pub fn success(paddr: PhysAddr, cycles: u64) -> Self {
        Self {
            paddr,
            cycles,
            trap: None,
        }
    }

    /// Creates a translation result indicating a fault occurred.
    ///
    /// # Arguments
    ///
    /// * `trap` - The trap that occurred during translation.
    /// * `cycles` - Number of cycles consumed before the fault.
    ///
    /// # Returns
    ///
    /// A `TranslationResult` indicating translation failure.
    #[inline]
    pub fn fault(trap: Trap, cycles: u64) -> Self {
        Self {
            paddr: PhysAddr(0),
            cycles,
            trap: Some(trap),
        }
    }
}
