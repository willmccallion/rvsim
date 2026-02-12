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
    pub(crate) fn csr_read(&self, addr: u32) -> u64 {
        match addr {
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
            csr::CYCLE | csr::MCYCLE => self.stats.cycles,
            csr::TIME => self.stats.cycles / self.clint_divider,
            csr::INSTRET | csr::MINSTRET => self.stats.instructions_retired,
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
            csr::CSR_SIM_PANIC => {
                self.trap(Trap::RequestedTrap(val), self.pc);
            }
            csr::MSTATUS => {
                self.csrs.mstatus = val;

                let mask = csr::MSTATUS_SIE
                    | csr::MSTATUS_SPIE
                    | csr::MSTATUS_SPP
                    | csr::MSTATUS_FS
                    | csr::MSTATUS_SUM
                    | csr::MSTATUS_MXR;
                self.csrs.sstatus = val & mask;
                self.interrupt_inhibit_one_cycle = true;
            }
            csr::MEDELEG => self.csrs.medeleg = val,
            csr::MIDELEG => self.csrs.mideleg = val,
            csr::MIE => {
                self.csrs.mie = val;
                self.interrupt_inhibit_one_cycle = true;
            }
            csr::MTVEC => self.csrs.mtvec = val,
            csr::MISA => self.csrs.misa = val,
            csr::MSCRATCH => self.csrs.mscratch = val,
            csr::MEPC => self.csrs.mepc = val & !1,
            csr::MCAUSE => self.csrs.mcause = val,
            csr::MTVAL => self.csrs.mtval = val,
            csr::MIP => {
                let mask = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
                self.csrs.mip = (self.csrs.mip & !mask) | (val & mask);
            }
            csr::SSTATUS => {
                let mask = csr::MSTATUS_SIE
                    | csr::MSTATUS_SPIE
                    | csr::MSTATUS_SPP
                    | csr::MSTATUS_FS
                    | csr::MSTATUS_SUM
                    | csr::MSTATUS_MXR;

                self.csrs.mstatus = (self.csrs.mstatus & !mask) | (val & mask);
                self.csrs.sstatus = self.csrs.mstatus & mask;
                self.interrupt_inhibit_one_cycle = true;
            }
            csr::SIE => {
                let mask = self.csrs.mideleg;
                self.csrs.mie = (self.csrs.mie & !mask) | (val & mask);
                self.interrupt_inhibit_one_cycle = true;
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
                self.flush_pipeline_stores();
                self.l1_d_cache.flush();

                self.mmu.dtlb.flush();
                self.mmu.itlb.flush();
            }
            _ => {}
        }
    }
}
