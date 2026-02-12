//! Trap Handling Utilities.
//!
//! This module provides helper functions for managing processor traps. It performs
//! the following:
//! 1. **Interrupt Mapping:** Converts hardware interrupt pending bits into high-level trap types.
//! 2. **Standardization:** Ensures consistent trap representation across the simulator.

use crate::common::error::Trap;

/// Trap handler utility functions.
///
/// Provides a unified interface for converting low-level interrupt signals into
/// architectural trap variants.
pub struct TrapHandler;

impl TrapHandler {
    /// Converts an interrupt pending bit to a corresponding trap type.
    ///
    /// # Arguments
    ///
    /// * `bit` - The interrupt pending bit from the `MIP` register.
    ///
    /// # Returns
    ///
    /// The `Trap` variant corresponding to the interrupt type. Defaults to
    /// `MachineTimerInterrupt` for unrecognized bits.
    pub fn irq_to_trap(bit: u64) -> Trap {
        use crate::core::arch::csr;
        match bit {
            csr::MIP_USIP => Trap::UserSoftwareInterrupt,
            csr::MIP_SSIP => Trap::SupervisorSoftwareInterrupt,
            csr::MIP_MSIP => Trap::MachineSoftwareInterrupt,
            csr::MIP_STIP => Trap::SupervisorTimerInterrupt,
            csr::MIP_MTIP => Trap::MachineTimerInterrupt,
            csr::MIP_UEIP => Trap::UserExternalInterrupt,
            csr::MIP_SEIP => Trap::SupervisorExternalInterrupt,
            csr::MIP_MEIP => Trap::MachineExternalInterrupt,
            _ => Trap::MachineTimerInterrupt,
        }
    }
}
