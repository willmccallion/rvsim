//! Trap Handling Utilities.

use crate::arch::Hart;
use crate::isa::csr;
use crate::isa::privileged::PrivilegeMode;
use crate::isa::privileged::Trap;

/// Converts an interrupt pending bit to a corresponding trap type.
/// Defaults to `MachineTimerInterrupt` for unrecognized bits.
pub const fn irq_to_trap(bit: u64) -> Trap {
    use crate::isa::csr;
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

impl Hart {
    /// Executes the `MRET` instruction (Return from Machine Mode).
    pub(crate) const fn do_mret(&mut self) {
        self.pc = self.csrs.mepc & !crate::arch::csr::ialign_low_bits(self.csrs.misa);
        let mstatus = self.csrs.mstatus;
        let mpp = (mstatus >> csr::MSTATUS_MPP_SHIFT) & csr::MSTATUS_MPP_MASK;
        let mpie = (mstatus & csr::MSTATUS_MPIE) != 0;

        self.privilege = PrivilegeMode::from_u8(mpp as u8);
        let mut new_mstatus = mstatus;
        if mpie {
            new_mstatus |= csr::MSTATUS_MIE;
        } else {
            new_mstatus &= !csr::MSTATUS_MIE;
        }
        new_mstatus |= csr::MSTATUS_MPIE;
        new_mstatus &= !csr::MSTATUS_MPP;
        // Per spec 3.1.6.1: if xPP != M, xRET also sets MPRV=0
        if mpp != PrivilegeMode::Machine.to_u8() as u64 {
            new_mstatus &= !csr::MSTATUS_MPRV;
        }

        self.csrs.mstatus = new_mstatus;
    }

    /// Executes the `SRET` instruction (Return from Supervisor Mode).
    pub(crate) const fn do_sret(&mut self) {
        self.pc = self.csrs.sepc & !crate::arch::csr::ialign_low_bits(self.csrs.misa);
        let mstatus = self.csrs.mstatus;
        let spp = (mstatus & csr::MSTATUS_SPP) != 0;
        let spie = (mstatus & csr::MSTATUS_SPIE) != 0;

        self.privilege = if spp { PrivilegeMode::Supervisor } else { PrivilegeMode::User };
        let mut new_mstatus = mstatus;
        if spie {
            new_mstatus |= csr::MSTATUS_SIE;
        } else {
            new_mstatus &= !csr::MSTATUS_SIE;
        }
        new_mstatus |= csr::MSTATUS_SPIE;
        new_mstatus &= !csr::MSTATUS_SPP;
        // Per spec 3.1.6.1: SRET returns to S or U (never M), so always clear MPRV
        new_mstatus &= !csr::MSTATUS_MPRV;
        self.csrs.mstatus = new_mstatus;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isa::csr;

    #[test]
    fn test_irq_to_trap() {
        assert_eq!(irq_to_trap(csr::MIP_USIP), Trap::UserSoftwareInterrupt);
        assert_eq!(irq_to_trap(csr::MIP_SSIP), Trap::SupervisorSoftwareInterrupt);
        assert_eq!(irq_to_trap(csr::MIP_MSIP), Trap::MachineSoftwareInterrupt);
        assert_eq!(irq_to_trap(csr::MIP_STIP), Trap::SupervisorTimerInterrupt);
        assert_eq!(irq_to_trap(csr::MIP_MTIP), Trap::MachineTimerInterrupt);
        assert_eq!(irq_to_trap(csr::MIP_UEIP), Trap::UserExternalInterrupt);
        assert_eq!(irq_to_trap(csr::MIP_SEIP), Trap::SupervisorExternalInterrupt);
        assert_eq!(irq_to_trap(csr::MIP_MEIP), Trap::MachineExternalInterrupt);

        assert_eq!(irq_to_trap(999), Trap::MachineTimerInterrupt);
    }
}
