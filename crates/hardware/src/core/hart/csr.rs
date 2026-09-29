//! CSR address validation and Sdtrig (debug trigger) lookups.

use super::Hart;
use crate::core::arch::csr;
use crate::isa::csr::CsrAddr;
use crate::isa::privileged::PrivilegeMode;

impl Hart {
    /// Returns `true` if the given CSR address corresponds to a CSR that is
    /// implemented by this hart. Accessing a non-existent CSR must raise an
    /// illegal-instruction exception (RISC-V Privileged Spec §2.2).
    pub const fn is_valid_csr(&self, addr: CsrAddr) -> bool {
        let raw = addr.as_u32();
        matches!(raw,
            x if x == csr::FFLAGS.as_u32()
                || x == csr::FRM.as_u32()
                || x == csr::FCSR.as_u32()
                || x == csr::MVENDORID.as_u32()
                || x == csr::MARCHID.as_u32()
                || x == csr::MIMPID.as_u32()
                || x == csr::MHARTID.as_u32()
                || x == csr::MSTATUS.as_u32()
                || x == csr::MEDELEG.as_u32()
                || x == csr::MIDELEG.as_u32()
                || x == csr::MIE.as_u32()
                || x == csr::MTVEC.as_u32()
                || x == csr::MISA.as_u32()
                || x == csr::MSCRATCH.as_u32()
                || x == csr::MEPC.as_u32()
                || x == csr::MCAUSE.as_u32()
                || x == csr::MTVAL.as_u32()
                || x == csr::MIP.as_u32()
                || x == csr::SSTATUS.as_u32()
                || x == csr::SIE.as_u32()
                || x == csr::STVEC.as_u32()
                || x == csr::SSCRATCH.as_u32()
                || x == csr::SEPC.as_u32()
                || x == csr::SCAUSE.as_u32()
                || x == csr::STVAL.as_u32()
                || x == csr::SIP.as_u32()
                || x == csr::STIMECMP.as_u32()
                || x == csr::SATP.as_u32()
                || x == csr::MCOUNTEREN.as_u32()
                || x == csr::SCOUNTEREN.as_u32()
                || x == csr::MENVCFG.as_u32()
                || x == csr::SENVCFG.as_u32()
                || x == csr::CYCLE.as_u32()
                || x == csr::MCYCLE.as_u32()
                || x == csr::TIME.as_u32()
                || x == csr::INSTRET.as_u32()
                || x == csr::MINSTRET.as_u32()
                || x == csr::VSTART.as_u32()
                || x == csr::VXSAT.as_u32()
                || x == csr::VXRM.as_u32()
                || x == csr::VCSR.as_u32()
                || x == csr::VL.as_u32()
                || x == csr::VTYPE.as_u32()
                || x == csr::VLENB.as_u32()
                || x == csr::CSR_SIM_PANIC.as_u32()
                // Sdtrig (debug trigger) stubs — read-zero / write-ignored
                || x == csr::TSELECT.as_u32()
                || x == csr::TDATA1.as_u32()
                || x == csr::TDATA2.as_u32()
                || x == csr::TDATA3.as_u32()
                || x == csr::TINFO.as_u32()
                || x == csr::TCONTROL.as_u32()
        ) || matches!(raw,
            x if x == csr::PMPCFG0.as_u32()
                || x == csr::PMPCFG2.as_u32()
                || x == csr::MCOUNTINHIBIT.as_u32()
        ) || (raw >= csr::PMPADDR0.as_u32() && raw <= csr::PMPADDR15.as_u32())
            || (raw >= csr::MHPMEVENT3.as_u32() && raw <= csr::MHPMEVENT31.as_u32())
            || (raw >= csr::MHPMCOUNTER3.as_u32() && raw <= csr::MHPMCOUNTER31.as_u32())
    }

    /// Returns true if an execute trigger fires for the given PC and current privilege.
    pub fn check_execute_trigger(&self, pc: u64) -> bool {
        let mte = (self.csrs.tcontrol >> 3) & 1 != 0;
        for i in 0..2usize {
            let tdata1 = self.csrs.tdata1[i];
            if (tdata1 >> 60) & 0xF != 2 {
                continue;
            } // not mcontrol
            if (tdata1 >> 9) & 1 == 0 {
                continue;
            } // not execute trigger
            let action = (tdata1 >> 19) & 0x3;
            if action != 0 {
                continue;
            } // only breakpoint exception
            let mode_ok = match self.privilege {
                PrivilegeMode::Machine => (tdata1 >> 13) & 1 != 0 && mte,
                PrivilegeMode::Supervisor => (tdata1 >> 11) & 1 != 0,
                PrivilegeMode::User => (tdata1 >> 10) & 1 != 0,
            };
            if mode_ok && self.csrs.tdata2[i] == pc {
                return true;
            }
        }
        false
    }

    /// Returns true if a load trigger fires for the given address and current privilege.
    pub fn check_load_trigger(&self, addr: u64) -> bool {
        let mte = (self.csrs.tcontrol >> 3) & 1 != 0;
        for i in 0..2usize {
            let tdata1 = self.csrs.tdata1[i];
            if (tdata1 >> 60) & 0xF != 2 {
                continue;
            }
            if (tdata1 >> 7) & 1 == 0 {
                continue;
            } // not load trigger
            let action = (tdata1 >> 19) & 0x3;
            if action != 0 {
                continue;
            }
            let mode_ok = match self.privilege {
                PrivilegeMode::Machine => (tdata1 >> 13) & 1 != 0 && mte,
                PrivilegeMode::Supervisor => (tdata1 >> 11) & 1 != 0,
                PrivilegeMode::User => (tdata1 >> 10) & 1 != 0,
            };
            if mode_ok && self.csrs.tdata2[i] == addr {
                return true;
            }
        }
        false
    }

    /// Returns true if a store trigger fires for the given address and current privilege.
    pub fn check_store_trigger(&self, addr: u64) -> bool {
        let mte = (self.csrs.tcontrol >> 3) & 1 != 0;
        for i in 0..2usize {
            let tdata1 = self.csrs.tdata1[i];
            if (tdata1 >> 60) & 0xF != 2 {
                continue;
            }
            if (tdata1 >> 8) & 1 == 0 {
                continue;
            } // not store trigger
            let action = (tdata1 >> 19) & 0x3;
            if action != 0 {
                continue;
            }
            let mode_ok = match self.privilege {
                PrivilegeMode::Machine => (tdata1 >> 13) & 1 != 0 && mte,
                PrivilegeMode::Supervisor => (tdata1 >> 11) & 1 != 0,
                PrivilegeMode::User => (tdata1 >> 10) & 1 != 0,
            };
            if mode_ok && self.csrs.tdata2[i] == addr {
                return true;
            }
        }
        false
    }
}
