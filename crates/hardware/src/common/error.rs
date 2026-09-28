//! Trap and Translation Result definitions.

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trap {
    /// Instruction address misaligned exception (carries misaligned address).
    InstructionAddressMisaligned(u64),
    /// Instruction access fault exception (carries faulting address).
    InstructionAccessFault(u64),
    /// Illegal instruction exception (carries instruction encoding).
    IllegalInstruction(u32),
    /// Breakpoint exception (carries program counter).
    Breakpoint(u64),
    /// Load address misaligned exception (carries misaligned address).
    LoadAddressMisaligned(u64),
    /// Load access fault exception (carries faulting address).
    LoadAccessFault(u64),
    /// Store address misaligned exception (carries misaligned address).
    StoreAddressMisaligned(u64),
    /// Store access fault exception (carries faulting address).
    StoreAccessFault(u64),
    /// Environment call from user mode.
    EnvironmentCallFromUMode,
    /// Environment call from supervisor mode.
    EnvironmentCallFromSMode,
    /// Environment call from machine mode.
    EnvironmentCallFromMMode,
    /// Instruction page fault exception (carries faulting virtual address).
    InstructionPageFault(u64),
    /// Load page fault exception (carries faulting virtual address).
    LoadPageFault(u64),
    /// Store page fault exception (carries faulting virtual address).
    StorePageFault(u64),
    /// User software interrupt.
    UserSoftwareInterrupt,
    /// Supervisor software interrupt.
    SupervisorSoftwareInterrupt,
    /// Machine software interrupt.
    MachineSoftwareInterrupt,
    /// Machine timer interrupt.
    MachineTimerInterrupt,
    /// Supervisor timer interrupt.
    SupervisorTimerInterrupt,
    /// Machine external interrupt.
    MachineExternalInterrupt,
    /// Supervisor external interrupt.
    SupervisorExternalInterrupt,
    /// User external interrupt.
    UserExternalInterrupt,
    /// Requested trap for debugging or simulation purposes (carries trap code).
    RequestedTrap(u64),
    /// Double fault — fault while handling another fault (carries faulting address).
    DoubleFault(u64),
}

impl fmt::Display for Trap {
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
    /// `(is_interrupt, code)` as `mcause` encodes it, without the interrupt bit.
    #[must_use]
    pub const fn cause(&self) -> (bool, u64) {
        use crate::common::constants::CAUSE_INTERRUPT_BIT;
        use crate::isa::privileged::cause::{exception, interrupt};
        match *self {
            Self::InstructionAddressMisaligned(_) => {
                (false, exception::INSTRUCTION_ADDRESS_MISALIGNED)
            }
            Self::InstructionAccessFault(_) => (false, exception::INSTRUCTION_ACCESS_FAULT),
            Self::IllegalInstruction(_) => (false, exception::ILLEGAL_INSTRUCTION),
            Self::Breakpoint(_) => (false, exception::BREAKPOINT),
            Self::LoadAddressMisaligned(_) => (false, exception::LOAD_ADDRESS_MISALIGNED),
            Self::LoadAccessFault(_) => (false, exception::LOAD_ACCESS_FAULT),
            Self::StoreAddressMisaligned(_) => (false, exception::STORE_ADDRESS_MISALIGNED),
            Self::StoreAccessFault(_) => (false, exception::STORE_ACCESS_FAULT),
            Self::EnvironmentCallFromUMode => (false, exception::ENVIRONMENT_CALL_FROM_U_MODE),
            Self::EnvironmentCallFromSMode => (false, exception::ENVIRONMENT_CALL_FROM_S_MODE),
            Self::EnvironmentCallFromMMode => (false, exception::ENVIRONMENT_CALL_FROM_M_MODE),
            Self::InstructionPageFault(_) => (false, exception::INSTRUCTION_PAGE_FAULT),
            Self::LoadPageFault(_) => (false, exception::LOAD_PAGE_FAULT),
            Self::StorePageFault(_) => (false, exception::STORE_PAGE_FAULT),
            Self::UserSoftwareInterrupt => (true, interrupt::USER_SOFTWARE & !CAUSE_INTERRUPT_BIT),
            Self::SupervisorSoftwareInterrupt => {
                (true, interrupt::SUPERVISOR_SOFTWARE & !CAUSE_INTERRUPT_BIT)
            }
            Self::MachineSoftwareInterrupt => {
                (true, interrupt::MACHINE_SOFTWARE & !CAUSE_INTERRUPT_BIT)
            }
            Self::SupervisorTimerInterrupt => {
                (true, interrupt::SUPERVISOR_TIMER & !CAUSE_INTERRUPT_BIT)
            }
            Self::MachineTimerInterrupt => (true, interrupt::MACHINE_TIMER & !CAUSE_INTERRUPT_BIT),
            Self::UserExternalInterrupt => (true, interrupt::USER_EXTERNAL & !CAUSE_INTERRUPT_BIT),
            Self::SupervisorExternalInterrupt => {
                (true, interrupt::SUPERVISOR_EXTERNAL & !CAUSE_INTERRUPT_BIT)
            }
            Self::MachineExternalInterrupt => {
                (true, interrupt::MACHINE_EXTERNAL & !CAUSE_INTERRUPT_BIT)
            }
            Self::RequestedTrap(c) => (false, c),
            Self::DoubleFault(_) => (false, exception::HARDWARE_ERROR),
        }
    }

    /// The full `mcause` value: the code with the interrupt bit set for interrupts.
    #[must_use]
    pub const fn mcause_code(&self) -> u64 {
        let (is_interrupt, code) = self.cause();
        if is_interrupt { code | crate::common::constants::CAUSE_INTERRUPT_BIT } else { code }
    }

    /// Timer interrupts and environment calls: the traps a running OS takes
    /// constantly, hidden from the trace unless asked for by cause.
    #[must_use]
    pub const fn is_routine(&self) -> bool {
        matches!(
            self,
            Self::MachineTimerInterrupt
                | Self::SupervisorTimerInterrupt
                | Self::EnvironmentCallFromUMode
                | Self::EnvironmentCallFromSMode
                | Self::EnvironmentCallFromMMode
        )
    }

    /// Returns the exception priority per RISC-V Privileged Spec Table 3.7.
    ///
    /// Lower values indicate higher priority. Synchronous exceptions have
    /// priorities 0-11, while interrupts have priority 12+.
    pub const fn exception_priority(&self) -> u8 {
        match self {
            Self::Breakpoint(_) => 0,
            Self::InstructionPageFault(_) => 1,
            Self::InstructionAccessFault(_) => 2,
            Self::IllegalInstruction(_) => 3,
            Self::InstructionAddressMisaligned(_) => 4,
            Self::EnvironmentCallFromUMode
            | Self::EnvironmentCallFromSMode
            | Self::EnvironmentCallFromMMode => 5,
            Self::StoreAddressMisaligned(_) => 6,
            Self::LoadAddressMisaligned(_) => 7,
            Self::StorePageFault(_) => 8,
            Self::LoadPageFault(_) => 9,
            Self::StoreAccessFault(_) => 10,
            Self::LoadAccessFault(_) => 11,
            Self::MachineExternalInterrupt => 12,
            Self::MachineSoftwareInterrupt => 13,
            Self::MachineTimerInterrupt => 14,
            Self::SupervisorExternalInterrupt => 15,
            Self::SupervisorSoftwareInterrupt => 16,
            Self::SupervisorTimerInterrupt => 17,
            Self::UserExternalInterrupt => 18,
            Self::UserSoftwareInterrupt => 19,
            Self::RequestedTrap(_) => 20,
            Self::DoubleFault(_) => 21,
        }
    }
}

impl std::error::Error for Trap {}

/// Bits a hardware A/D update sets in a leaf PTE.
///
/// The update is conditional (privileged spec, Svadu): it applies only while
/// the PTE still holds the value the walk checked, apart from its A/D bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PteUpdate {
    /// Physical address of the PTE in memory.
    pub pte_addr: crate::common::PhysAddr,
    /// The PTE the walk checked.
    pub walked_pte: u64,
    /// The A/D bits to set.
    pub set_bits: u64,
}

/// The D-bit updates a store applies when it retires: one for each page it
/// writes (two when it crosses a page boundary).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyUpdates([Option<PteUpdate>; 2]);

impl DirtyUpdates {
    /// No update: the access sets no D bit.
    pub const NONE: Self = Self([None, None]);

    /// The updates for the access's first page and, when it crosses into
    /// one, its second.
    #[must_use]
    pub const fn of(first: Option<PteUpdate>, second: Option<PteUpdate>) -> Self {
        Self([first, second])
    }

    /// True when the access sets no D bit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Every update, first page first.
    pub fn iter(&self) -> impl Iterator<Item = &PteUpdate> {
        self.0.iter().flatten()
    }
}

/// The accessed (A) and dirty (D) bits of a PTE.
const PTE_AD_BITS: u64 = 0b1100_0000;

impl PteUpdate {
    /// The PTE to write over `current`, or `None` when the entry has changed
    /// since the walk in more than its A/D bits and the access must
    /// translate again.
    #[must_use]
    pub const fn applied_to(&self, current: u64) -> Option<u64> {
        if (current ^ self.walked_pte) & !PTE_AD_BITS != 0 {
            return None;
        }
        Some(current | self.set_bits)
    }
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
}

/// Result of a virtual-to-physical address translation operation.
///
/// This structure encapsulates the outcome of an MMU walk, including performance
/// metrics and any faults that may have occurred.
#[derive(Clone, Debug)]
pub struct TranslationResult {
    /// The translated physical address, or zero if translation failed.
    pub paddr: PhysAddr,
    /// Number of cycles consumed by the translation operation.
    pub cycles: u64,
    /// Trap that occurred during translation, if any.
    pub trap: Option<Trap>,
    /// The D-bit update a store applies when it retires (it must be exact).
    pub dirty_update: Option<PteUpdate>,
    /// The A-bit update the walker applies at once (it may be speculative).
    pub accessed_update: Option<PteUpdate>,
}

impl TranslationResult {
    /// Creates a successful translation result.
    #[inline]
    pub const fn success(paddr: PhysAddr, cycles: u64) -> Self {
        Self { paddr, cycles, trap: None, dirty_update: None, accessed_update: None }
    }

    /// Creates a translation result indicating a fault occurred.
    #[inline]
    pub const fn fault(trap: Trap, cycles: u64) -> Self {
        Self {
            paddr: PhysAddr(0),
            cycles,
            trap: Some(trap),
            dirty_update: None,
            accessed_update: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: u64 = 1 << 7;
    const A: u64 = 1 << 6;
    const LEAF: u64 = (0x8_0400 << 10) | 0b0100_0111;

    fn set_dirty() -> PteUpdate {
        PteUpdate { pte_addr: PhysAddr(0x8010_2000), walked_pte: LEAF, set_bits: A | D }
    }

    #[test]
    fn a_pte_update_sets_its_bits_on_the_walked_pte() {
        assert_eq!(set_dirty().applied_to(LEAF), Some(LEAF | D));
    }

    #[test]
    fn a_pte_update_ignores_a_and_d_changes_since_the_walk() {
        assert_eq!(set_dirty().applied_to(LEAF & !A), Some(LEAF | D));
    }

    #[test]
    fn a_pte_update_is_refused_once_the_mapping_changed() {
        let remapped = LEAF + (1 << 10);
        let revoked = LEAF & !0b100;

        assert_eq!(
            (set_dirty().applied_to(remapped), set_dirty().applied_to(revoked)),
            (None, None)
        );
    }

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
