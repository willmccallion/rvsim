//! Traps: the exceptions and interrupts the privileged architecture defines.

use std::fmt;

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
        use crate::isa::privileged::cause::CAUSE_INTERRUPT_BIT;
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
        if is_interrupt { code | crate::isa::privileged::cause::CAUSE_INTERRUPT_BIT } else { code }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
