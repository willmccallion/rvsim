//! CSR Access Logic with read/write side effects (TLB flushes, interrupt synchronization).

use super::SimState;
use crate::common::{CsrAddr, Trap};
use crate::core::arch::csr;

impl SimState {
    /// Returns `true` if the given CSR address corresponds to a CSR that is
    /// implemented by this hart.
    #[inline]
    pub const fn is_valid_csr(&self, addr: CsrAddr) -> bool {
        self.hart.is_valid_csr(addr)
    }

    /// Reads a value from a Control and Status Register (CSR).
    pub fn csr_read(&self, addr: CsrAddr) -> u64 {
        let raw = addr.as_u32();
        match raw {
            x if x == csr::FFLAGS.as_u32() => self.hart.csrs.fflags & 0x1F,
            x if x == csr::FRM.as_u32() => self.hart.csrs.frm & 0x7,
            x if x == csr::FCSR.as_u32() => {
                ((self.hart.csrs.frm & 0x7) << 5) | (self.hart.csrs.fflags & 0x1F)
            }
            x if x == csr::MVENDORID.as_u32()
                || x == csr::MARCHID.as_u32()
                || x == csr::MIMPID.as_u32() =>
            {
                0
            }
            x if x == csr::MHARTID.as_u32() => u64::from(self.hart.hart_id.val()),
            x if x == csr::MSTATUS.as_u32() => {
                let val = self.hart.csrs.mstatus & !csr::MSTATUS_SD;
                if val & csr::MSTATUS_FS == csr::MSTATUS_FS_DIRTY {
                    val | csr::MSTATUS_SD
                } else {
                    val
                }
            }
            x if x == csr::MEDELEG.as_u32() => self.hart.csrs.medeleg,
            x if x == csr::MIDELEG.as_u32() => self.hart.csrs.mideleg,
            x if x == csr::MIE.as_u32() => self.hart.csrs.mie,
            x if x == csr::MTVEC.as_u32() => self.hart.csrs.mtvec,
            x if x == csr::MISA.as_u32() => self.hart.csrs.misa,
            x if x == csr::MSCRATCH.as_u32() => self.hart.csrs.mscratch,
            x if x == csr::MEPC.as_u32() => self.hart.csrs.mepc,
            x if x == csr::MCAUSE.as_u32() => self.hart.csrs.mcause,
            x if x == csr::MTVAL.as_u32() => self.hart.csrs.mtval,
            x if x == csr::MIP.as_u32() => self.hart.csrs.mip,
            x if x == csr::SSTATUS.as_u32() => {
                let val = self.hart.csrs.sstatus & !csr::MSTATUS_SD;
                if val & csr::MSTATUS_FS == csr::MSTATUS_FS_DIRTY {
                    val | csr::MSTATUS_SD
                } else {
                    val
                }
            }
            x if x == csr::SIE.as_u32() => self.hart.csrs.mie & self.hart.csrs.mideleg,
            x if x == csr::STVEC.as_u32() => self.hart.csrs.stvec,
            x if x == csr::SSCRATCH.as_u32() => self.hart.csrs.sscratch,
            x if x == csr::SEPC.as_u32() => self.hart.csrs.sepc,
            x if x == csr::SCAUSE.as_u32() => self.hart.csrs.scause,
            x if x == csr::STVAL.as_u32() => self.hart.csrs.stval,
            x if x == csr::SIP.as_u32() => self.hart.csrs.mip & self.hart.csrs.mideleg,
            x if x == csr::STIMECMP.as_u32() => self.hart.csrs.stimecmp,
            x if x == csr::SATP.as_u32() => self.hart.csrs.satp,
            x if x == csr::MCOUNTEREN.as_u32() => self.hart.csrs.mcounteren,
            x if x == csr::SCOUNTEREN.as_u32() => self.hart.csrs.scounteren,
            x if x == csr::MENVCFG.as_u32() => self.hart.csrs.menvcfg,
            x if x == csr::SENVCFG.as_u32() => self.hart.csrs.senvcfg,
            x if x == csr::CYCLE.as_u32() || x == csr::MCYCLE.as_u32() => self.cycle,
            x if x == csr::TIME.as_u32() => self.cycle / self.config.system.clint_divider,
            x if x == csr::INSTRET.as_u32() || x == csr::MINSTRET.as_u32() => {
                self.instructions_retired
            }
            x if x == csr::PMPCFG0.as_u32() => {
                self.hart.pmp.get_cfg(0) as u64
                    | ((self.hart.pmp.get_cfg(1) as u64) << 8)
                    | ((self.hart.pmp.get_cfg(2) as u64) << 16)
                    | ((self.hart.pmp.get_cfg(3) as u64) << 24)
                    | ((self.hart.pmp.get_cfg(4) as u64) << 32)
                    | ((self.hart.pmp.get_cfg(5) as u64) << 40)
                    | ((self.hart.pmp.get_cfg(6) as u64) << 48)
                    | ((self.hart.pmp.get_cfg(7) as u64) << 56)
            }
            x if x == csr::PMPCFG2.as_u32() => {
                self.hart.pmp.get_cfg(8) as u64
                    | ((self.hart.pmp.get_cfg(9) as u64) << 8)
                    | ((self.hart.pmp.get_cfg(10) as u64) << 16)
                    | ((self.hart.pmp.get_cfg(11) as u64) << 24)
                    | ((self.hart.pmp.get_cfg(12) as u64) << 32)
                    | ((self.hart.pmp.get_cfg(13) as u64) << 40)
                    | ((self.hart.pmp.get_cfg(14) as u64) << 48)
                    | ((self.hart.pmp.get_cfg(15) as u64) << 56)
            }
            x if x >= csr::PMPADDR0.as_u32() && x <= csr::PMPADDR15.as_u32() => {
                self.hart.pmp.get_addr((raw - csr::PMPADDR0.as_u32()) as usize)
            }
            // Vector CSRs (read-only: VL, VTYPE, VLENB; read-write: VSTART, VXSAT, VXRM, VCSR)
            x if x == csr::VSTART.as_u32() => self.hart.csrs.vstart,
            x if x == csr::VXSAT.as_u32() => self.hart.csrs.vxsat & 0x1,
            x if x == csr::VXRM.as_u32() => self.hart.csrs.vxrm & 0x3,
            x if x == csr::VCSR.as_u32() => (self.hart.csrs.vxsat & 0x1) | ((self.hart.csrs.vxrm & 0x3) << 1),
            x if x == csr::VL.as_u32() => self.hart.csrs.vl,
            x if x == csr::VTYPE.as_u32() => self.hart.csrs.vtype,
            x if x == csr::VLENB.as_u32() => self.hart.csrs.vlenb,
            // Sdtrig — trigger CSR reads
            x if x == csr::TSELECT.as_u32() => self.hart.csrs.tselect,
            x if x == csr::TDATA1.as_u32() => {
                let i = self.hart.csrs.tselect as usize;
                self.hart.csrs.tdata1[i]
            }
            x if x == csr::TDATA2.as_u32() => {
                let i = self.hart.csrs.tselect as usize;
                self.hart.csrs.tdata2[i]
            }
            x if x == csr::TDATA3.as_u32() => 0, // not implemented
            x if x == csr::TINFO.as_u32() => 1 << 2, // mcontrol supported
            x if x == csr::TCONTROL.as_u32() => self.hart.csrs.tcontrol & 0x88, // mte=bit3, mpte=bit7
            _ => 0,
        }
    }

    /// Writes a value to a Control and Status Register (CSR).
    pub fn csr_write(&mut self, addr: CsrAddr, val: u64) {
        let raw = addr.as_u32();
        match raw {
            x if x == csr::FFLAGS.as_u32() => {
                self.hart.csrs.fflags = val & 0x1F;
                self.hart.csrs.mstatus = (self.hart.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                self.hart.csrs.sstatus = (self.hart.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            }
            x if x == csr::FRM.as_u32() => {
                self.hart.csrs.frm = val & 0x7;
                self.hart.csrs.mstatus = (self.hart.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                self.hart.csrs.sstatus = (self.hart.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            }
            x if x == csr::FCSR.as_u32() => {
                self.hart.csrs.fflags = val & 0x1F;
                self.hart.csrs.frm = (val >> 5) & 0x7;
                self.hart.csrs.mstatus = (self.hart.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                self.hart.csrs.sstatus = (self.hart.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            }
            x if x == csr::CSR_SIM_PANIC.as_u32() => {
                self.trap(&Trap::RequestedTrap(val), self.hart.pc);
            }
            x if x == csr::MSTATUS.as_u32() => {
                // WARL: only defined writable bits are accepted; WPRI/SD/UXL/SXL ignored.
                const MSTATUS_WRITABLE: u64 = csr::MSTATUS_SIE
                    | csr::MSTATUS_MIE
                    | csr::MSTATUS_SPIE
                    | csr::MSTATUS_MPIE
                    | csr::MSTATUS_SPP
                    | csr::MSTATUS_MPP
                    | csr::MSTATUS_FS
                    | csr::MSTATUS_MPRV
                    | csr::MSTATUS_SUM
                    | csr::MSTATUS_MXR
                    | csr::MSTATUS_TVM
                    | csr::MSTATUS_TW
                    | csr::MSTATUS_TSR;
                // UXL and SXL are hardwired to 2 (RV64)
                let preserved = self.hart.csrs.mstatus & (csr::MSTATUS_UXL | csr::MSTATUS_SXL);
                self.hart.csrs.mstatus = (val & MSTATUS_WRITABLE) | preserved;

                // WARL: MPP must encode a supported privilege mode (0=U, 1=S, 3=M).
                // Value 2 is reserved; clamp to 0 (User) to prevent privilege escalation.
                let mpp = (self.hart.csrs.mstatus >> csr::MSTATUS_MPP_SHIFT) & csr::MSTATUS_MPP_MASK;
                if mpp == 2 {
                    self.hart.csrs.mstatus &= !csr::MSTATUS_MPP;
                }

                let mask = csr::MSTATUS_SIE
                    | csr::MSTATUS_SPIE
                    | csr::MSTATUS_SPP
                    | csr::MSTATUS_FS
                    | csr::MSTATUS_SUM
                    | csr::MSTATUS_MXR
                    | csr::MSTATUS_UXL;
                self.hart.csrs.sstatus = self.hart.csrs.mstatus & mask;
            }
            x if x == csr::MEDELEG.as_u32() => {
                // Bit 11 (ecall from M-mode) cannot be delegated
                self.hart.csrs.medeleg = val & !(1 << 11);
            }
            x if x == csr::MIDELEG.as_u32() => {
                // Only S-level interrupts can be delegated (not M-level)
                let mask = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
                self.hart.csrs.mideleg = val & mask;
            }
            x if x == csr::MIE.as_u32() => {
                // WARL: only defined interrupt-enable bits are writable
                let mask = csr::MIE_SSIP
                    | csr::MIE_MSIP
                    | csr::MIE_STIE
                    | csr::MIE_MTIE
                    | csr::MIE_SEIP
                    | csr::MIE_MEIP;
                self.hart.csrs.mie = val & mask;
            }
            x if x == csr::MTVEC.as_u32() => {
                // WARL: mode field (bits 1:0) only supports 0 (Direct) and 1 (Vectored).
                // Reserved modes (2, 3) are clamped to Direct by clearing both mode bits.
                let mode = val & 3;
                self.hart.csrs.mtvec = if mode >= 2 { val & !3 } else { val };
            }
            x if x == csr::MISA.as_u32() => {
                // MISA is WARL: writes are silently ignored (extensions are hardwired).
            }
            x if x == csr::MSCRATCH.as_u32() => self.hart.csrs.mscratch = val,
            x if x == csr::MEPC.as_u32() => self.hart.csrs.mepc = val & !1,
            x if x == csr::MCAUSE.as_u32() => self.hart.csrs.mcause = val,
            x if x == csr::MTVAL.as_u32() => self.hart.csrs.mtval = val,
            x if x == csr::MIP.as_u32() => {
                let mask = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
                self.hart.csrs.mip = (self.hart.csrs.mip & !mask) | (val & mask);
                // Track software-written SEIP so pre_tick preserves it
                self.hart.sw_seip = (val & csr::MIP_SEIP) != 0;
            }
            x if x == csr::SSTATUS.as_u32() => {
                // UXL is read-only in sstatus (always reflects mstatus UXL)
                let writable_mask = csr::MSTATUS_SIE
                    | csr::MSTATUS_SPIE
                    | csr::MSTATUS_SPP
                    | csr::MSTATUS_FS
                    | csr::MSTATUS_SUM
                    | csr::MSTATUS_MXR;
                let read_mask = writable_mask | csr::MSTATUS_UXL;

                self.hart.csrs.mstatus = (self.hart.csrs.mstatus & !writable_mask) | (val & writable_mask);
                self.hart.csrs.sstatus = self.hart.csrs.mstatus & read_mask;
            }
            x if x == csr::SIE.as_u32() => {
                let mask = self.hart.csrs.mideleg;
                self.hart.csrs.mie = (self.hart.csrs.mie & !mask) | (val & mask);
            }
            x if x == csr::STVEC.as_u32() => {
                // WARL: mode field (bits 1:0) only supports 0 (Direct) and 1 (Vectored).
                let mode = val & 3;
                self.hart.csrs.stvec = if mode >= 2 { val & !3 } else { val };
            }
            x if x == csr::SSCRATCH.as_u32() => self.hart.csrs.sscratch = val,
            x if x == csr::SEPC.as_u32() => self.hart.csrs.sepc = val & !1,
            x if x == csr::SCAUSE.as_u32() => self.hart.csrs.scause = val,
            x if x == csr::STVAL.as_u32() => self.hart.csrs.stval = val,
            x if x == csr::SIP.as_u32() => {
                let mask = self.hart.csrs.mideleg & (csr::MIP_SSIP);
                self.hart.csrs.mip = (self.hart.csrs.mip & !mask) | (val & mask);
            }
            x if x == csr::MCOUNTEREN.as_u32() => {
                // Only CY(0), TM(1), IR(2) are implemented
                self.hart.csrs.mcounteren = val & 0x7;
            }
            x if x == csr::SCOUNTEREN.as_u32() => {
                self.hart.csrs.scounteren = val & 0x7;
            }
            x if x == csr::MENVCFG.as_u32() => {
                self.hart.csrs.menvcfg = val;
            }
            x if x == csr::SENVCFG.as_u32() => {
                self.hart.csrs.senvcfg = val;
            }
            x if x == csr::MCYCLE.as_u32() => self.cycle = val,
            x if x == csr::MINSTRET.as_u32() => self.instructions_retired = val,
            x if x == csr::PMPCFG0.as_u32() => {
                for i in 0..8 {
                    self.hart.pmp.set_cfg(i, ((val >> (i * 8)) & 0xFF) as u8);
                }
            }
            x if x == csr::PMPCFG2.as_u32() => {
                for i in 0..8 {
                    self.hart.pmp.set_cfg(8 + i, ((val >> (i * 8)) & 0xFF) as u8);
                }
            }
            x if x >= csr::PMPADDR0.as_u32() && x <= csr::PMPADDR15.as_u32() => {
                self.hart.pmp.set_addr((raw - csr::PMPADDR0.as_u32()) as usize, val);
            }
            x if x == csr::STIMECMP.as_u32() => {
                self.hart.csrs.stimecmp = val;
                self.hart.csrs.mip &= !csr::MIP_STIP;
            }
            x if x == csr::SATP.as_u32() => {
                let mode = (val >> csr::SATP_MODE_SHIFT) & csr::SATP_MODE_MASK;
                let allowed = csr::PagingMode::from_satp_mode(mode)
                    .is_some_and(|m| m.is_at_most(self.hart.mmu.paging_mode_max));

                let new_val = if allowed {
                    val
                } else {
                    val & !(csr::SATP_MODE_MASK << csr::SATP_MODE_SHIFT)
                };

                self.hart.csrs.satp = new_val;

                let _ = self.core.l1_i_cache.invalidate_all();
                let _ = self.core.l1_d_cache.flush();

                self.hart.mmu.dtlb.flush();
                self.hart.mmu.itlb.flush();
                self.hart.mmu.l2_tlb.flush();
            }
            // Writable vector CSRs
            x if x == csr::VSTART.as_u32() => self.hart.csrs.vstart = val,
            x if x == csr::VXSAT.as_u32() => self.hart.csrs.vxsat = val & 0x1,
            x if x == csr::VXRM.as_u32() => self.hart.csrs.vxrm = val & 0x3,
            x if x == csr::VCSR.as_u32() => {
                self.hart.csrs.vxsat = val & 0x1;
                self.hart.csrs.vxrm = (val >> 1) & 0x3;
            }
            // VL, VTYPE, VLENB are read-only (writes silently ignored)
            // Sdtrig — trigger CSR writes
            x if x == csr::TSELECT.as_u32() => {
                // WARL: clamp to valid trigger index
                self.hart.csrs.tselect = val.min(1); // MAX_TRIGGERS-1 = 1
            }
            x if x == csr::TDATA1.as_u32() => {
                let i = self.hart.csrs.tselect as usize;
                let ttype = (val >> 60) & 0xF;
                if ttype == 2 {
                    // mcontrol: accept supported fields, force action=0, dmode=0
                    const MCONTROL_MASK: u64 = (0xFu64 << 60) // type
                        | (1 << 13) | (1 << 11) | (1 << 10)   // m, s, u
                        | (1 << 9) | (1 << 8) | (1 << 7); // execute, store, load
                    self.hart.csrs.tdata1[i] = val & MCONTROL_MASK;
                } else {
                    // type=0 or unsupported: disable trigger
                    self.hart.csrs.tdata1[i] = 0;
                }
            }
            x if x == csr::TDATA2.as_u32() => {
                let i = self.hart.csrs.tselect as usize;
                self.hart.csrs.tdata2[i] = val;
            }
            x if x == csr::TDATA3.as_u32() => {} // not implemented
            x if x == csr::TINFO.as_u32() => {}  // read-only
            x if x == csr::TCONTROL.as_u32() => {
                self.hart.csrs.tcontrol = val & 0x88; // only mte (bit3) and mpte (bit7)
            }
            _ => {}
        }
    }

    /// Returns true if an execute trigger fires for the given PC and current privilege.
    #[inline]
    pub fn check_execute_trigger(&self, pc: u64) -> bool {
        self.hart.check_execute_trigger(pc)
    }

    /// Returns true if a load trigger fires for the given address and current privilege.
    #[inline]
    pub fn check_load_trigger(&self, addr: u64) -> bool {
        self.hart.check_load_trigger(addr)
    }

    /// Returns true if a store trigger fires for the given address and current privilege.
    #[inline]
    pub fn check_store_trigger(&self, addr: u64) -> bool {
        self.hart.check_store_trigger(addr)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::sim::SimState;
    use crate::core::arch::csr;

    #[test]
    fn test_cpu_csr_read_write_mstatus() {
        let config = Config::default();
        let mut cpu = SimState::build(&config, "");

        cpu.csr_write(csr::MSTATUS, 0xFFFF_FFFF_FFFF_FFFF);

        let mstatus = cpu.csr_read(csr::MSTATUS);
        assert_ne!(mstatus, 0xFFFF_FFFF_FFFF_FFFF);

        let sstatus = cpu.csr_read(csr::SSTATUS);
        assert_eq!(
            sstatus,
            mstatus
                & (csr::MSTATUS_SD
                    | csr::MSTATUS_SIE
                    | csr::MSTATUS_SPIE
                    | csr::MSTATUS_SPP
                    | csr::MSTATUS_FS
                    | csr::MSTATUS_SUM
                    | csr::MSTATUS_MXR
                    | csr::MSTATUS_UXL)
        );
    }

    #[test]
    fn test_cpu_csr_read_write_fcsr() {
        let config = Config::default();
        let mut cpu = SimState::build(&config, "");

        cpu.csr_write(csr::FCSR, 0xFF);
        assert_eq!(cpu.csr_read(csr::FCSR), 0xFF);
        assert_eq!(cpu.csr_read(csr::FFLAGS), 0x1F);
        assert_eq!(cpu.csr_read(csr::FRM), 0x7);
    }
}
