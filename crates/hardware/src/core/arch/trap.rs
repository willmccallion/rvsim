//! Trap Handling Utilities.

use crate::isa::privileged::Trap;

/// Trap handler utility functions.
///
/// Provides a unified interface for converting low-level interrupt signals into
/// architectural trap variants.
#[derive(Debug)]
pub struct TrapHandler;

impl TrapHandler {
    /// Converts an interrupt pending bit to a corresponding trap type.
    /// Defaults to `MachineTimerInterrupt` for unrecognized bits.
    pub const fn irq_to_trap(bit: u64) -> Trap {
        use crate::core::arch::csr;
        match bit {
            csr::MIP_USIP => Trap::UserSoftwareInterrupt,
            csr::MIP_SSIP => Trap::SupervisorSoftwareInterrupt,
            csr::MIP_MSIP => Trap::MachineSoftwareInterrupt,
            csr::MIP_STIP => Trap::SupervisorTimerInterrupt,
            csr::MIP_UEIP => Trap::UserExternalInterrupt,
            csr::MIP_SEIP => Trap::SupervisorExternalInterrupt,
            csr::MIP_MEIP => Trap::MachineExternalInterrupt,
            _ => Trap::MachineTimerInterrupt,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::arch::csr;

    #[test]
    fn test_irq_to_trap() {
        assert_eq!(TrapHandler::irq_to_trap(csr::MIP_USIP), Trap::UserSoftwareInterrupt);
        assert_eq!(TrapHandler::irq_to_trap(csr::MIP_SSIP), Trap::SupervisorSoftwareInterrupt);
        assert_eq!(TrapHandler::irq_to_trap(csr::MIP_MSIP), Trap::MachineSoftwareInterrupt);
        assert_eq!(TrapHandler::irq_to_trap(csr::MIP_STIP), Trap::SupervisorTimerInterrupt);
        assert_eq!(TrapHandler::irq_to_trap(csr::MIP_MTIP), Trap::MachineTimerInterrupt);
        assert_eq!(TrapHandler::irq_to_trap(csr::MIP_UEIP), Trap::UserExternalInterrupt);
        assert_eq!(TrapHandler::irq_to_trap(csr::MIP_SEIP), Trap::SupervisorExternalInterrupt);
        assert_eq!(TrapHandler::irq_to_trap(csr::MIP_MEIP), Trap::MachineExternalInterrupt);

        assert_eq!(TrapHandler::irq_to_trap(999), Trap::MachineTimerInterrupt);
    }
}
