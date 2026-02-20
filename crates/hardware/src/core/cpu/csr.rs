//! CSR Access Logic.
//!
//! This module implements the Control and Status Register (CSR) access mechanisms for the CPU.
//! It performs the following:
//! 1. **Read Operations:** Retrieves CSR values while handling architectural side effects.
//! 2. **Write Operations:** Updates CSR state and triggers necessary system updates (e.g., TLB flushes).
//! 3. **Side Effect Management:** Handles interrupt inhibition and status bit synchronization.

use super::Cpu;
use crate::common::Trap;
use crate::core::arch::csr;

impl Cpu {
    /// Reads a value from a Control and Status Register (CSR).
    ///
    /// # Arguments
    ///
    /// * `addr` - The 12-bit address of the CSR to read.
    ///
    /// # Returns
    ///
    /// The current 64-bit value of the specified CSR.
    pub fn csr_read(&self, addr: u32) -> u64 {
        match addr {
            csr::FFLAGS => self.csrs.fflags & 0x1F,
            csr::FRM => self.csrs.frm & 0x7,
            csr::FCSR => ((self.csrs.frm & 0x7) << 5) | (self.csrs.fflags & 0x1F),
            csr::MVENDORID => 0,
            csr::MARCHID => 0,
            csr::MIMPID => 0,
            csr::MHARTID => 0,
            csr::MSTATUS => self.csrs.mstatus,
            csr::MEDELEG => self.csrs.medeleg,
            csr::MIDELEG => self.csrs.mideleg,
            csr::MIE => self.csrs.mie,
            csr::MTVEC => self.csrs.mtvec,
            csr::MISA => self.csrs.misa,
            csr::MSCRATCH => self.csrs.mscratch,
            csr::MEPC => self.csrs.mepc,
            csr::MCAUSE => self.csrs.mcause,
            csr::MTVAL => self.csrs.mtval,
            csr::MIP => self.csrs.mip,
            csr::SSTATUS => self.csrs.sstatus,
            csr::SIE => self.csrs.mie & self.csrs.mideleg,
            csr::STVEC => self.csrs.stvec,
            csr::SSCRATCH => self.csrs.sscratch,
            csr::SEPC => self.csrs.sepc,
            csr::SCAUSE => self.csrs.scause,
            csr::STVAL => self.csrs.stval,
            csr::SIP => self.csrs.mip & self.csrs.mideleg,
            csr::STIMECMP => self.csrs.stimecmp,
            csr::SATP => self.csrs.satp,
            csr::MCOUNTEREN => self.csrs.mcounteren,
            csr::SCOUNTEREN => self.csrs.scounteren,
            csr::CYCLE | csr::MCYCLE => self.stats.cycles,
            csr::TIME => self.stats.cycles / self.clint_divider,
            csr::INSTRET | csr::MINSTRET => self.stats.instructions_retired,
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
            0x3B0..=0x3BF => self.pmp.get_addr((addr - 0x3B0) as usize),
            _ => 0,
        }
    }

    /// Writes a value to a Control and Status Register (CSR).
    ///
    /// # Arguments
    ///
    /// * `addr` - The 12-bit address of the CSR to write.
    /// * `val` - The 64-bit value to write to the register.
    pub fn csr_write(&mut self, addr: u32, val: u64) {
        match addr {
            csr::FFLAGS => self.csrs.fflags = val & 0x1F,
            csr::FRM => self.csrs.frm = val & 0x7,
            csr::FCSR => {
                self.csrs.fflags = val & 0x1F;
                self.csrs.frm = (val >> 5) & 0x7;
            }
            csr::CSR_SIM_PANIC => {
                self.trap(Trap::RequestedTrap(val), self.pc);
            }
            csr::MSTATUS => {
                // WARL: preserve UXL and SXL (bits 35:32) — always 2 (64-bit) on RV64.
                let uxl_sxl_mask: u64 = 0xF << 32;
                let preserved = self.csrs.mstatus & uxl_sxl_mask;
                self.csrs.mstatus = (val & !uxl_sxl_mask) | preserved;

                let mask = csr::MSTATUS_SIE
                    | csr::MSTATUS_SPIE
                    | csr::MSTATUS_SPP
                    | csr::MSTATUS_FS
                    | csr::MSTATUS_SUM
                    | csr::MSTATUS_MXR
                    | csr::MSTATUS_UXL;
                self.csrs.sstatus = self.csrs.mstatus & mask;
            }
            csr::MEDELEG => self.csrs.medeleg = val,
            csr::MIDELEG => self.csrs.mideleg = val,
            csr::MIE => {
                self.csrs.mie = val;
            }
            csr::MTVEC => self.csrs.mtvec = val,
            csr::MISA => {
                // MISA is WARL: writes are silently ignored (extensions are hardwired).
            }
            csr::MSCRATCH => self.csrs.mscratch = val,
            csr::MEPC => self.csrs.mepc = val & !1,
            csr::MCAUSE => self.csrs.mcause = val,
            csr::MTVAL => self.csrs.mtval = val,
            csr::MIP => {
                let mask = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
                self.csrs.mip = (self.csrs.mip & !mask) | (val & mask);
            }
            csr::SSTATUS => {
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
            csr::SIE => {
                let mask = self.csrs.mideleg;
                self.csrs.mie = (self.csrs.mie & !mask) | (val & mask);
            }
            csr::STVEC => {
                self.csrs.stvec = val;
            }
            csr::SSCRATCH => self.csrs.sscratch = val,
            csr::SEPC => self.csrs.sepc = val & !1,
            csr::SCAUSE => self.csrs.scause = val,
            csr::STVAL => self.csrs.stval = val,
            csr::SIP => {
                let mask = self.csrs.mideleg & (csr::MIP_SSIP);
                self.csrs.mip = (self.csrs.mip & !mask) | (val & mask);
            }
            csr::MCOUNTEREN => self.csrs.mcounteren = val,
            csr::SCOUNTEREN => self.csrs.scounteren = val,
            csr::MCYCLE => self.stats.cycles = val,
            csr::MINSTRET => self.stats.instructions_retired = val,
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
                self.pmp.set_addr((addr - 0x3B0) as usize, val);
            }
            csr::STIMECMP => {
                self.csrs.stimecmp = val;
                self.csrs.mip &= !csr::MIP_STIP;
            }
            csr::SATP => {
                let mode = (val >> csr::SATP_MODE_SHIFT) & csr::SATP_MODE_MASK;

                let new_val = if mode == csr::SATP_MODE_SV39 || mode == csr::SATP_MODE_BARE {
                    val
                } else {
                    val & !(csr::SATP_MODE_MASK << csr::SATP_MODE_SHIFT)
                };

                self.csrs.satp = new_val;
                self.clear_reservation(); // SATP write invalidates reservations

                // Flush BOTH instruction and data caches
                self.l1_i_cache.flush();
                self.l1_d_cache.flush();

                self.mmu.dtlb.flush();
                self.mmu.itlb.flush();
            }
            _ => {}
        }
    }
}
