//! Trap return operations: MRET (return from M-mode) and SRET (return from S-mode).

use super::Hart;
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;

impl Hart {
    /// Executes the `MRET` instruction (Return from Machine Mode).
    pub(crate) const fn do_mret(&mut self) {
        self.pc = self.csrs.mepc & !1;
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
        self.pc = self.csrs.sepc & !1;
        let sstatus = self.csrs.sstatus;
        let spp = (sstatus & csr::MSTATUS_SPP) != 0;
        let spie = (sstatus & csr::MSTATUS_SPIE) != 0;

        self.privilege = if spp { PrivilegeMode::Supervisor } else { PrivilegeMode::User };
        let mut new_sstatus = sstatus;
        if spie {
            new_sstatus |= csr::MSTATUS_SIE;
        } else {
            new_sstatus &= !csr::MSTATUS_SIE;
        }
        new_sstatus |= csr::MSTATUS_SPIE;
        new_sstatus &= !csr::MSTATUS_SPP;

        self.csrs.sstatus = new_sstatus;
        let mask = csr::MSTATUS_SIE | csr::MSTATUS_SPIE | csr::MSTATUS_SPP;
        let mut new_mstatus = (self.csrs.mstatus & !mask) | (new_sstatus & mask);
        // Per spec 3.1.6.1: SRET returns to S or U (never M), so always clear MPRV
        new_mstatus &= !csr::MSTATUS_MPRV;
        self.csrs.mstatus = new_mstatus;
    }
}
