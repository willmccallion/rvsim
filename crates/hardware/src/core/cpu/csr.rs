//! CSR Access Logic.
//!
//! This module implements the Control and Status Register (CSR) access mechanisms for the CPU.
//! It performs the following:
//! 1. **Read Operations:** Retrieves CSR values while handling architectural side effects.
//! 2. **Write Operations:** Updates CSR state and triggers necessary system updates (e.g., TLB flushes).
//! 3. **Side Effect Management:** Handles interrupt inhibition and status bit synchronization.

use super::Cpu;
use crate::common::{CsrAddr, Trap};
use crate::core::arch::csr;

impl Cpu {
    /// Reads a value from a Control and Status Register (CSR).
    ///
    /// # Arguments
    ///
    /// * `addr` - The CSR address.
    ///
    /// # Returns
    ///
    /// The current 64-bit value of the specified CSR.
    pub fn csr_read(&self, addr: CsrAddr) -> u64 {
        let raw = addr.as_u32();
        match raw {
            x if x == csr::FFLAGS.as_u32() => self.csrs.fflags & 0x1F,
            x if x == csr::FRM.as_u32() => self.csrs.frm & 0x7,
            x if x == csr::FCSR.as_u32() => {
                ((self.csrs.frm & 0x7) << 5) | (self.csrs.fflags & 0x1F)
            }
            x if x == csr::MVENDORID.as_u32()
                || x == csr::MARCHID.as_u32()
                || x == csr::MIMPID.as_u32()
                || x == csr::MHARTID.as_u32() =>
            {
                0
            }
            x if x == csr::MSTATUS.as_u32() => {
                let val = self.csrs.mstatus & !csr::MSTATUS_SD;
                if val & csr::MSTATUS_FS == csr::MSTATUS_FS_DIRTY {
                    val | csr::MSTATUS_SD
                } else {
                    val
                }
            }
            x if x == csr::MEDELEG.as_u32() => self.csrs.medeleg,
            x if x == csr::MIDELEG.as_u32() => self.csrs.mideleg,
            x if x == csr::MIE.as_u32() => self.csrs.mie,
            x if x == csr::MTVEC.as_u32() => self.csrs.mtvec,
            x if x == csr::MISA.as_u32() => self.csrs.misa,
            x if x == csr::MSCRATCH.as_u32() => self.csrs.mscratch,
            x if x == csr::MEPC.as_u32() => self.csrs.mepc,
            x if x == csr::MCAUSE.as_u32() => self.csrs.mcause,
            x if x == csr::MTVAL.as_u32() => self.csrs.mtval,
            x if x == csr::MIP.as_u32() => self.csrs.mip,
            x if x == csr::SSTATUS.as_u32() => {
                let val = self.csrs.sstatus & !csr::MSTATUS_SD;
                if val & csr::MSTATUS_FS == csr::MSTATUS_FS_DIRTY {
                    val | csr::MSTATUS_SD
                } else {
                    val
                }
            }
            x if x == csr::SIE.as_u32() => self.csrs.mie & self.csrs.mideleg,
            x if x == csr::STVEC.as_u32() => self.csrs.stvec,
            x if x == csr::SSCRATCH.as_u32() => self.csrs.sscratch,
            x if x == csr::SEPC.as_u32() => self.csrs.sepc,
            x if x == csr::SCAUSE.as_u32() => self.csrs.scause,
            x if x == csr::STVAL.as_u32() => self.csrs.stval,
            x if x == csr::SIP.as_u32() => self.csrs.mip & self.csrs.mideleg,
            x if x == csr::STIMECMP.as_u32() => self.csrs.stimecmp,
            x if x == csr::SATP.as_u32() => self.csrs.satp,
            x if x == csr::MCOUNTEREN.as_u32() => self.csrs.mcounteren,
            x if x == csr::SCOUNTEREN.as_u32() => self.csrs.scounteren,
            x if x == csr::MENVCFG.as_u32() => self.csrs.menvcfg,
            x if x == csr::CYCLE.as_u32() || x == csr::MCYCLE.as_u32() => self.stats.cycles,
            x if x == csr::TIME.as_u32() => self.stats.cycles / self.clint_divider,
            x if x == csr::INSTRET.as_u32() || x == csr::MINSTRET.as_u32() => {
                self.stats.instructions_retired
            }
            0x3A0 => {
                self.pmp.get_cfg(0) as u64
                    | ((self.pmp.get_cfg(1) as u64) << 8)
                    | ((self.pmp.get_cfg(2) as u64) << 16)
                    | ((self.pmp.get_cfg(3) as u64) << 24)
                    | ((self.pmp.get_cfg(4) as u64) << 32)
                    | ((self.pmp.get_cfg(5) as u64) << 40)
                    | ((self.pmp.get_cfg(6) as u64) << 48)
                    | ((self.pmp.get_cfg(7) as u64) << 56)
            }
            0x3A2 => {
                self.pmp.get_cfg(8) as u64
                    | ((self.pmp.get_cfg(9) as u64) << 8)
                    | ((self.pmp.get_cfg(10) as u64) << 16)
                    | ((self.pmp.get_cfg(11) as u64) << 24)
                    | ((self.pmp.get_cfg(12) as u64) << 32)
                    | ((self.pmp.get_cfg(13) as u64) << 40)
                    | ((self.pmp.get_cfg(14) as u64) << 48)
                    | ((self.pmp.get_cfg(15) as u64) << 56)
            }
            0x3B0..=0x3BF => self.pmp.get_addr((raw - 0x3B0) as usize),
            // Vector CSRs (read-only: VL, VTYPE, VLENB; read-write: VSTART, VXSAT, VXRM, VCSR)
            x if x == csr::VSTART.as_u32() => self.csrs.vstart,
            x if x == csr::VXSAT.as_u32() => self.csrs.vxsat & 0x1,
            x if x == csr::VXRM.as_u32() => self.csrs.vxrm & 0x3,
            x if x == csr::VCSR.as_u32() => (self.csrs.vxsat & 0x1) | ((self.csrs.vxrm & 0x3) << 1),
            x if x == csr::VL.as_u32() => self.csrs.vl,
            x if x == csr::VTYPE.as_u32() => self.csrs.vtype,
            x if x == csr::VLENB.as_u32() => self.csrs.vlenb,
            _ => 0,
        }
    }

    /// Writes a value to a Control and Status Register (CSR).
    ///
    /// # Arguments
    ///
    /// * `addr` - The CSR address.
    /// * `val` - The 64-bit value to write to the register.
    pub fn csr_write(&mut self, addr: CsrAddr, val: u64) {
        let raw = addr.as_u32();
        match raw {
            x if x == csr::FFLAGS.as_u32() => {
                self.csrs.fflags = val & 0x1F;
                self.csrs.mstatus = (self.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                self.csrs.sstatus = (self.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            }
            x if x == csr::FRM.as_u32() => {
                self.csrs.frm = val & 0x7;
                self.csrs.mstatus = (self.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                self.csrs.sstatus = (self.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            }
            x if x == csr::FCSR.as_u32() => {
                self.csrs.fflags = val & 0x1F;
                self.csrs.frm = (val >> 5) & 0x7;
                self.csrs.mstatus = (self.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                self.csrs.sstatus = (self.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
            }
            x if x == csr::CSR_SIM_PANIC.as_u32() => {
                self.trap(&Trap::RequestedTrap(val), self.pc);
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
                let preserved = self.csrs.mstatus & (csr::MSTATUS_UXL | csr::MSTATUS_SXL);
                self.csrs.mstatus = (val & MSTATUS_WRITABLE) | preserved;

                // WARL: MPP must encode a supported privilege mode (0=U, 1=S, 3=M).
                // Value 2 is reserved; clamp to 0 (User) to prevent privilege escalation.
                let mpp = (self.csrs.mstatus >> csr::MSTATUS_MPP_SHIFT) & csr::MSTATUS_MPP_MASK;
                if mpp == 2 {
                    self.csrs.mstatus &= !csr::MSTATUS_MPP;
                }

                let mask = csr::MSTATUS_SIE
                    | csr::MSTATUS_SPIE
                    | csr::MSTATUS_SPP
                    | csr::MSTATUS_FS
                    | csr::MSTATUS_SUM
                    | csr::MSTATUS_MXR
                    | csr::MSTATUS_UXL;
                self.csrs.sstatus = self.csrs.mstatus & mask;
            }
            x if x == csr::MEDELEG.as_u32() => {
                // Bit 11 (ecall from M-mode) cannot be delegated
                self.csrs.medeleg = val & !(1 << 11);
            }
            x if x == csr::MIDELEG.as_u32() => {
                // Only S-level interrupts can be delegated (not M-level)
                let mask = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
                self.csrs.mideleg = val & mask;
            }
            x if x == csr::MIE.as_u32() => {
                // WARL: only defined interrupt-enable bits are writable
                let mask = csr::MIE_SSIP
                    | csr::MIE_MSIP
                    | csr::MIE_STIE
                    | csr::MIE_MTIE
                    | csr::MIE_SEIP
                    | csr::MIE_MEIP;
                self.csrs.mie = val & mask;
            }
            x if x == csr::MTVEC.as_u32() => {
                // WARL: mode field (bits 1:0) only supports 0 (Direct) and 1 (Vectored).
                // Reserved modes (2, 3) are clamped to Direct by clearing both mode bits.
                let mode = val & 3;
                self.csrs.mtvec = if mode >= 2 { val & !3 } else { val };
            }
            x if x == csr::MISA.as_u32() => {
                // MISA is WARL: writes are silently ignored (extensions are hardwired).
            }
            x if x == csr::MSCRATCH.as_u32() => self.csrs.mscratch = val,
            x if x == csr::MEPC.as_u32() => self.csrs.mepc = val & !1,
            x if x == csr::MCAUSE.as_u32() => self.csrs.mcause = val,
            x if x == csr::MTVAL.as_u32() => self.csrs.mtval = val,
            x if x == csr::MIP.as_u32() => {
                let mask = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
                self.csrs.mip = (self.csrs.mip & !mask) | (val & mask);
                // Track software-written SEIP so pre_tick preserves it
                self.sw_seip = (val & csr::MIP_SEIP) != 0;
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

                self.csrs.mstatus = (self.csrs.mstatus & !writable_mask) | (val & writable_mask);
                self.csrs.sstatus = self.csrs.mstatus & read_mask;
            }
            x if x == csr::SIE.as_u32() => {
                let mask = self.csrs.mideleg;
                self.csrs.mie = (self.csrs.mie & !mask) | (val & mask);
            }
            x if x == csr::STVEC.as_u32() => {
                // WARL: mode field (bits 1:0) only supports 0 (Direct) and 1 (Vectored).
                let mode = val & 3;
                self.csrs.stvec = if mode >= 2 { val & !3 } else { val };
            }
            x if x == csr::SSCRATCH.as_u32() => self.csrs.sscratch = val,
            x if x == csr::SEPC.as_u32() => self.csrs.sepc = val & !1,
            x if x == csr::SCAUSE.as_u32() => self.csrs.scause = val,
            x if x == csr::STVAL.as_u32() => self.csrs.stval = val,
            x if x == csr::SIP.as_u32() => {
                let mask = self.csrs.mideleg & (csr::MIP_SSIP);
                self.csrs.mip = (self.csrs.mip & !mask) | (val & mask);
            }
            x if x == csr::MCOUNTEREN.as_u32() => {
                // Only CY(0), TM(1), IR(2) are implemented
                self.csrs.mcounteren = val & 0x7;
            }
            x if x == csr::SCOUNTEREN.as_u32() => {
                self.csrs.scounteren = val & 0x7;
            }
            x if x == csr::MENVCFG.as_u32() => {
                self.csrs.menvcfg = val;
            }
            x if x == csr::MCYCLE.as_u32() => self.stats.cycles = val,
            x if x == csr::MINSTRET.as_u32() => self.stats.instructions_retired = val,
            0x3A0 => {
                for i in 0..8 {
                    self.pmp.set_cfg(i, ((val >> (i * 8)) & 0xFF) as u8);
                }
            }
            0x3A2 => {
                for i in 0..8 {
                    self.pmp.set_cfg(8 + i, ((val >> (i * 8)) & 0xFF) as u8);
                }
            }
            0x3B0..=0x3BF => {
                self.pmp.set_addr((raw - 0x3B0) as usize, val);
            }
            x if x == csr::STIMECMP.as_u32() => {
                self.csrs.stimecmp = val;
                self.csrs.mip &= !csr::MIP_STIP;
            }
            x if x == csr::SATP.as_u32() => {
                let mode = (val >> csr::SATP_MODE_SHIFT) & csr::SATP_MODE_MASK;

                let new_val = if mode == csr::SATP_MODE_SV39 || mode == csr::SATP_MODE_BARE {
                    val
                } else {
                    val & !(csr::SATP_MODE_MASK << csr::SATP_MODE_SHIFT)
                };

                self.csrs.satp = new_val;

                // Flush BOTH instruction and data caches
                let _ = self.l1_i_cache.invalidate_all();
                let _ = self.l1_d_cache.flush();

                self.mmu.dtlb.flush();
                self.mmu.itlb.flush();
                self.mmu.l2_tlb.flush();
            }
            // Writable vector CSRs
            x if x == csr::VSTART.as_u32() => self.csrs.vstart = val,
            x if x == csr::VXSAT.as_u32() => self.csrs.vxsat = val & 0x1,
            x if x == csr::VXRM.as_u32() => self.csrs.vxrm = val & 0x3,
            x if x == csr::VCSR.as_u32() => {
                self.csrs.vxsat = val & 0x1;
                self.csrs.vxrm = (val >> 1) & 0x3;
            }
            // VL, VTYPE, VLENB are read-only (writes silently ignored)
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::core::Cpu;
    use crate::core::arch::csr;

    #[test]
    fn test_cpu_csr_read_write_mstatus() {
        let config = Config::default();
        let system = crate::soc::builder::System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.csr_write(csr::MSTATUS, 0xFFFF_FFFF_FFFF_FFFF);

        let mstatus = cpu.csr_read(csr::MSTATUS);
        assert_ne!(mstatus, 0xFFFF_FFFF_FFFF_FFFF); // specific bits are masked out or preserved

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
        let system = crate::soc::builder::System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.csr_write(csr::FCSR, 0xFF);
        assert_eq!(cpu.csr_read(csr::FCSR), 0xFF);
        assert_eq!(cpu.csr_read(csr::FFLAGS), 0x1F);
        assert_eq!(cpu.csr_read(csr::FRM), 0x7);
    }
}
