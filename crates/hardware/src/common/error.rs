//! Trap and Translation Result definitions.
//!
//! This module defines the error handling and trap mechanisms for the simulator. It provides:
//! 1. **Trap Representation:** Encompassing all synchronous exceptions and asynchronous interrupts.
//! 2. **Translation Results:** Reporting the outcome of virtual-to-physical address translation.
//! 3. **Error Handling:** Integrating with standard Rust error traits for system-level reporting.

use std::fmt;

use super::addr::PhysAddr;
use super::reg_idx::RegIdx;

/// Pipeline stage where an exception was first detected.
///
/// Used to track exception origin through the pipeline for accurate
/// trap handling and diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ExceptionStage {
    /// Exception detected during instruction fetch.
    #[default]
    Fetch,
    /// Exception detected during instruction decode.
    Decode,
    /// Exception detected during execution.
    Execute,
    /// Exception detected during memory access.
    Memory,
}

/// RISC-V trap types representing exceptions and interrupts.
///
/// Traps cause the processor to transfer control to a predefined trap handler.
/// This enum covers all standard traps defined in the RISC-V Privileged Specification.
#[derive(Clone, Debug, PartialEq, Eq)]
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
            Self::InstructionAddressMisaligned(addr) => {
                write!(f, "InstructionAddressMisaligned({addr:#x})")
            }
            Self::InstructionAccessFault(addr) => {
                write!(f, "InstructionAccessFault({addr:#x})")
            }
            Self::IllegalInstruction(inst) => write!(f, "IllegalInstruction({inst:#x})"),
            Self::Breakpoint(pc) => write!(f, "Breakpoint({pc:#x})"),
            Self::LoadAddressMisaligned(addr) => write!(f, "LoadAddressMisaligned({addr:#x})"),
            Self::LoadAccessFault(addr) => write!(f, "LoadAccessFault({addr:#x})"),
            Self::StoreAddressMisaligned(addr) => {
                write!(f, "StoreAddressMisaligned({addr:#x})")
            }
            Self::StoreAccessFault(addr) => write!(f, "StoreAccessFault({addr:#x})"),
            Self::EnvironmentCallFromUMode => write!(f, "EnvironmentCallFromUMode"),
            Self::EnvironmentCallFromSMode => write!(f, "EnvironmentCallFromSMode"),
            Self::EnvironmentCallFromMMode => write!(f, "EnvironmentCallFromMMode"),
            Self::InstructionPageFault(addr) => write!(f, "InstructionPageFault({addr:#x})"),
            Self::LoadPageFault(addr) => write!(f, "LoadPageFault({addr:#x})"),
            Self::StorePageFault(addr) => write!(f, "StorePageFault({addr:#x})"),
            Self::UserSoftwareInterrupt => write!(f, "UserSoftwareInterrupt"),
            Self::SupervisorSoftwareInterrupt => write!(f, "SupervisorSoftwareInterrupt"),
            Self::MachineSoftwareInterrupt => write!(f, "MachineSoftwareInterrupt"),
            Self::MachineTimerInterrupt => write!(f, "MachineTimerInterrupt"),
            Self::SupervisorTimerInterrupt => write!(f, "SupervisorTimerInterrupt"),
            Self::MachineExternalInterrupt => write!(f, "MachineExternalInterrupt"),
            Self::SupervisorExternalInterrupt => write!(f, "SupervisorExternalInterrupt"),
            Self::UserExternalInterrupt => write!(f, "UserExternalInterrupt"),
            Self::RequestedTrap(code) => write!(f, "RequestedTrap({code})"),
            Self::DoubleFault(addr) => write!(f, "DoubleFault({addr:#x})"),
        }
    }
}

impl Trap {
    /// Returns the exception priority per RISC-V Privileged Spec Table 3.7.
    ///
    /// Lower values indicate higher priority. Synchronous exceptions have
    /// priorities 0-11, while interrupts have priority 12+.
    pub const fn exception_priority(&self) -> u8 {
        match self {
            // Highest priority: instruction address breakpoint
            Self::Breakpoint(_) => 0,

            // Instruction fetch exceptions
            Self::InstructionPageFault(_) => 1,
            Self::InstructionAccessFault(_) => 2,

            // Illegal instruction / decode errors
            Self::IllegalInstruction(_) => 3,
            Self::InstructionAddressMisaligned(_) => 4,

            // Environment calls
            Self::EnvironmentCallFromUMode
            | Self::EnvironmentCallFromSMode
            | Self::EnvironmentCallFromMMode => 5,

            // Store/AMO address misaligned
            Self::StoreAddressMisaligned(_) => 6,
            Self::LoadAddressMisaligned(_) => 7,

            // Store/AMO page fault & access fault
            Self::StorePageFault(_) => 8,
            Self::LoadPageFault(_) => 9,
            Self::StoreAccessFault(_) => 10,
            Self::LoadAccessFault(_) => 11,

            // Interrupts (lower priority than synchronous exceptions)
            Self::MachineExternalInterrupt => 12,
            Self::MachineSoftwareInterrupt => 13,
            Self::MachineTimerInterrupt => 14,
            Self::SupervisorExternalInterrupt => 15,
            Self::SupervisorSoftwareInterrupt => 16,
            Self::SupervisorTimerInterrupt => 17,
            Self::UserExternalInterrupt => 18,
            Self::UserSoftwareInterrupt => 19,

            // Simulator-specific traps (lowest priority)
            Self::RequestedTrap(_) => 20,
            Self::DoubleFault(_) => 21,
        }
    }
}

impl std::error::Error for Trap {}

/// Deferred PTE Accessed/Dirty bit update.
///
/// During page table walks, the PTW computes updated A/D bits but does NOT
/// write them to memory immediately — the instruction may be speculative.
/// Instead, the update is carried through the pipeline and applied at commit.
#[derive(Clone, Copy, Debug, Default)]
pub struct PteUpdate {
    /// Physical address of the PTE in memory.
    pub pte_addr: crate::common::PhysAddr,
    /// New PTE value with A/D bits set.
    pub pte_value: u64,
}

/// Deferred SFENCE.VMA operands for commit-time TLB invalidation.
///
/// SFENCE.VMA must not take effect speculatively — preceding PTE-modifying
/// stores may still be in the store buffer at execute time.  The operand
/// values are captured at execute and carried through the pipeline so the
/// commit stage can perform the correct selective (or global) TLB flush
/// after the store buffer has fully drained.
#[derive(Clone, Copy, Debug, Default)]
pub struct SfenceVmaInfo {
    /// Architectural index of rs1 (0 = flush all virtual addresses).
    pub rs1_idx: RegIdx,
    /// Architectural index of rs2 (0 = flush all ASIDs).
    pub rs2_idx: RegIdx,
    /// Value of rs1 (virtual address, when `rs1_idx` != 0).
    pub rs1_val: u64,
    /// Value of rs2 (ASID, when `rs2_idx` != 0).
    pub rs2_val: u64,
}

/// Deferred LR/SC reservation action for commit-time application.
///
/// LR/SC must not modify the load reservation speculatively — if the
/// instruction is squashed, the reservation state would be corrupted.
/// Instead, Memory2 records the intended action here, and the commit
/// stage applies it when the instruction retires.
#[derive(Clone, Copy, Debug)]
pub enum LrScRecord {
    /// LR: set the reservation to this physical address at commit.
    Lr {
        /// Physical address to reserve.
        paddr: crate::common::PhysAddr,
    },
    /// SC: check the reservation at commit.  If valid, clear it and
    /// let the store drain.  If invalid, the speculative SC result (0)
    /// was wrong — cancel the store and flush from this instruction.
    Sc {
        /// Physical address to check reservation against.
        paddr: crate::common::PhysAddr,
    },
}

/// Result of a virtual-to-physical address translation operation.
///
/// This structure encapsulates the outcome of an MMU walk, including performance
/// metrics and any faults that may have occurred.
#[derive(Debug)]
pub struct TranslationResult {
    /// The translated physical address, or zero if translation failed.
    pub paddr: PhysAddr,
    /// Number of cycles consumed by the translation operation.
    pub cycles: u64,
    /// Trap that occurred during translation, if any.
    pub trap: Option<Trap>,
    /// Deferred PTE A/D bit update to apply at commit time.
    pub pte_update: Option<PteUpdate>,
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
    pub const fn success(paddr: PhysAddr, cycles: u64) -> Self {
        Self { paddr, cycles, trap: None, pte_update: None }
    }

    /// Creates a successful translation result with a deferred PTE update.
    #[inline]
    pub const fn success_with_pte_update(
        paddr: PhysAddr,
        cycles: u64,
        pte_update: PteUpdate,
    ) -> Self {
        Self { paddr, cycles, trap: None, pte_update: Some(pte_update) }
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
    pub const fn fault(trap: Trap, cycles: u64) -> Self {
        Self { paddr: PhysAddr(0), cycles, trap: Some(trap), pte_update: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exception_stage_default() {
        assert_eq!(ExceptionStage::default(), ExceptionStage::Fetch);
    }

    #[test]
    fn test_trap_display() {
        assert_eq!(
            format!("{}", Trap::InstructionAddressMisaligned(0x1000)),
            "InstructionAddressMisaligned(0x1000)"
        );
        assert_eq!(
            format!("{}", Trap::InstructionAccessFault(0x2000)),
            "InstructionAccessFault(0x2000)"
        );
        assert_eq!(format!("{}", Trap::IllegalInstruction(0x3000)), "IllegalInstruction(0x3000)");
        assert_eq!(format!("{}", Trap::Breakpoint(0x4000)), "Breakpoint(0x4000)");
        assert_eq!(
            format!("{}", Trap::LoadAddressMisaligned(0x5000)),
            "LoadAddressMisaligned(0x5000)"
        );
        assert_eq!(format!("{}", Trap::LoadAccessFault(0x6000)), "LoadAccessFault(0x6000)");
        assert_eq!(
            format!("{}", Trap::StoreAddressMisaligned(0x7000)),
            "StoreAddressMisaligned(0x7000)"
        );
        assert_eq!(format!("{}", Trap::StoreAccessFault(0x8000)), "StoreAccessFault(0x8000)");
        assert_eq!(format!("{}", Trap::EnvironmentCallFromUMode), "EnvironmentCallFromUMode");
        assert_eq!(format!("{}", Trap::EnvironmentCallFromSMode), "EnvironmentCallFromSMode");
        assert_eq!(format!("{}", Trap::EnvironmentCallFromMMode), "EnvironmentCallFromMMode");
        assert_eq!(
            format!("{}", Trap::InstructionPageFault(0x9000)),
            "InstructionPageFault(0x9000)"
        );
        assert_eq!(format!("{}", Trap::LoadPageFault(0xa000)), "LoadPageFault(0xa000)");
        assert_eq!(format!("{}", Trap::StorePageFault(0xb000)), "StorePageFault(0xb000)");
        assert_eq!(format!("{}", Trap::UserSoftwareInterrupt), "UserSoftwareInterrupt");
        assert_eq!(format!("{}", Trap::SupervisorSoftwareInterrupt), "SupervisorSoftwareInterrupt");
        assert_eq!(format!("{}", Trap::MachineSoftwareInterrupt), "MachineSoftwareInterrupt");
        assert_eq!(format!("{}", Trap::MachineTimerInterrupt), "MachineTimerInterrupt");
        assert_eq!(format!("{}", Trap::SupervisorTimerInterrupt), "SupervisorTimerInterrupt");
        assert_eq!(format!("{}", Trap::MachineExternalInterrupt), "MachineExternalInterrupt");
        assert_eq!(format!("{}", Trap::SupervisorExternalInterrupt), "SupervisorExternalInterrupt");
        assert_eq!(format!("{}", Trap::UserExternalInterrupt), "UserExternalInterrupt");
        assert_eq!(format!("{}", Trap::RequestedTrap(42)), "RequestedTrap(42)");
        assert_eq!(format!("{}", Trap::DoubleFault(0xc000)), "DoubleFault(0xc000)");
    }

    #[test]
    fn test_trap_priority() {
        assert_eq!(Trap::Breakpoint(0).exception_priority(), 0);
        assert_eq!(Trap::InstructionPageFault(0).exception_priority(), 1);
        assert_eq!(Trap::InstructionAccessFault(0).exception_priority(), 2);
        assert_eq!(Trap::IllegalInstruction(0).exception_priority(), 3);
        assert_eq!(Trap::InstructionAddressMisaligned(0).exception_priority(), 4);
        assert_eq!(Trap::EnvironmentCallFromUMode.exception_priority(), 5);
        assert_eq!(Trap::EnvironmentCallFromSMode.exception_priority(), 5);
        assert_eq!(Trap::EnvironmentCallFromMMode.exception_priority(), 5);
        assert_eq!(Trap::StoreAddressMisaligned(0).exception_priority(), 6);
        assert_eq!(Trap::LoadAddressMisaligned(0).exception_priority(), 7);
        assert_eq!(Trap::StorePageFault(0).exception_priority(), 8);
        assert_eq!(Trap::LoadPageFault(0).exception_priority(), 9);
        assert_eq!(Trap::StoreAccessFault(0).exception_priority(), 10);
        assert_eq!(Trap::LoadAccessFault(0).exception_priority(), 11);
        assert_eq!(Trap::MachineExternalInterrupt.exception_priority(), 12);
        assert_eq!(Trap::MachineSoftwareInterrupt.exception_priority(), 13);
        assert_eq!(Trap::MachineTimerInterrupt.exception_priority(), 14);
        assert_eq!(Trap::SupervisorExternalInterrupt.exception_priority(), 15);
        assert_eq!(Trap::SupervisorSoftwareInterrupt.exception_priority(), 16);
        assert_eq!(Trap::SupervisorTimerInterrupt.exception_priority(), 17);
        assert_eq!(Trap::UserExternalInterrupt.exception_priority(), 18);
        assert_eq!(Trap::UserSoftwareInterrupt.exception_priority(), 19);
        assert_eq!(Trap::RequestedTrap(0).exception_priority(), 20);
        assert_eq!(Trap::DoubleFault(0).exception_priority(), 21);
    }

    #[test]
    fn test_translation_result() {
        let success = TranslationResult::success(PhysAddr(0x1000), 5);
        assert_eq!(success.paddr, PhysAddr(0x1000));
        assert_eq!(success.cycles, 5);
        assert!(success.trap.is_none());

        let fault = TranslationResult::fault(Trap::LoadPageFault(0x2000), 3);
        assert_eq!(fault.paddr, PhysAddr(0));
        assert_eq!(fault.cycles, 3);
        assert_eq!(fault.trap, Some(Trap::LoadPageFault(0x2000)));
    }
}
