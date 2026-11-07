//! CSR Access Logic with read/write side effects (TLB flushes, interrupt synchronization).

use super::CoreCtx;
use crate::arch::Hart;
use crate::arch::csr::legalize_envcfg;
use crate::arch::csr::legalize_tvec;
use crate::arch::csr::with_state_dirty;
use crate::arch::pmp::{PmpCfgCsr, PmpIdx};
use crate::arch::trigger;
use crate::isa::csr;
use crate::isa::csr::CsrAddr;
use crate::isa::fp::{FpFlags, Frm};
use crate::isa::privileged::cause::exception;
use crate::isa::privileged::{Satp, Trap};
use crate::isa::rvv::Vxrm;
use crate::soc::uncore::Uncore;

/// The value a CSR read-modify-write starts from. `mip` reads SEIP as
/// the OR of the PLIC's line and the software bit, but only the
/// software bit takes part in a CSRRS/CSRRC (privileged spec §3.1.9):
/// otherwise clearing another bit while the line is high would set
/// the software bit and leave SEIP pending for good.
pub(super) fn read_for_update(hart: &Hart, uncore: &Uncore, addr: CsrAddr) -> u64 {
    let value = read(hart, uncore, addr);
    if addr != csr::MIP {
        return value;
    }
    let software_seip = if hart.sw_seip { csr::MIP_SEIP } else { 0 };
    (value & !csr::MIP_SEIP) | software_seip
}

/// Reads a value from a Control and Status Register (CSR).
pub(super) fn read(hart: &Hart, uncore: &Uncore, addr: CsrAddr) -> u64 {
    match addr {
        csr::FFLAGS => hart.csrs.fflags_bits(),
        csr::FRM => hart.csrs.frm.bits(),
        csr::FCSR => hart.csrs.fcsr(),
        csr::MVENDORID | csr::MARCHID | csr::MIMPID => 0,
        csr::MHARTID => u64::from(hart.hart_id.val()),
        csr::MSTATUS => with_state_dirty(hart.csrs.mstatus),
        csr::MEDELEG => hart.csrs.medeleg,
        csr::MIDELEG => hart.csrs.mideleg,
        csr::MIE => hart.csrs.mie,
        csr::MTVEC => hart.csrs.mtvec,
        csr::MISA => hart.csrs.misa.bits(),
        csr::MSCRATCH => hart.csrs.mscratch,
        csr::MEPC => hart.csrs.mepc,
        csr::MCAUSE => hart.csrs.mcause,
        csr::MTVAL => hart.csrs.mtval,
        csr::MIP => hart.csrs.mip,
        csr::SSTATUS => with_state_dirty(hart.csrs.sstatus()),
        csr::SIE => hart.csrs.mie & hart.csrs.mideleg,
        csr::STVEC => hart.csrs.stvec,
        csr::SSCRATCH => hart.csrs.sscratch,
        csr::SEPC => hart.csrs.sepc,
        csr::SCAUSE => hart.csrs.scause,
        csr::STVAL => hart.csrs.stval,
        csr::SIP => hart.csrs.mip & hart.csrs.mideleg,
        csr::STIMECMP => hart.csrs.stimecmp,
        csr::SATP => hart.csrs.satp,
        csr::MCOUNTEREN => hart.csrs.mcounteren,
        csr::SCOUNTEREN => hart.csrs.scounteren,
        csr::MENVCFG => hart.csrs.menvcfg,
        csr::SENVCFG => hart.csrs.senvcfg,
        csr::CYCLE | csr::MCYCLE => hart.csrs.mcycle,
        csr::TIME => uncore.bus.mtime(),
        csr::INSTRET | csr::MINSTRET => hart.csrs.minstret,
        csr::MCOUNTINHIBIT => hart.csrs.mcountinhibit,
        csr::PMPCFG0 => hart.pmp.cfg_word(PmpCfgCsr::Pmpcfg0),
        csr::PMPCFG2 => hart.pmp.cfg_word(PmpCfgCsr::Pmpcfg2),
        _ if let Some(index) = PmpIdx::of_pmpaddr(addr) => hart.pmp.get_addr(index),
        // Vector CSRs (read-only: VL, VTYPE, VLENB; read-write: VSTART, VXSAT, VXRM, VCSR)
        csr::VSTART => hart.csrs.vstart,
        csr::VXSAT => hart.csrs.vxsat & csr::VXSAT_BIT,
        csr::VXRM => hart.csrs.vxrm_bits(),
        csr::VCSR => hart.csrs.vcsr(),
        csr::VL => hart.csrs.vl,
        csr::VTYPE => hart.csrs.vtype,
        csr::VLENB => hart.csrs.vlenb,
        // Sdtrig — trigger CSR reads
        csr::TSELECT => hart.csrs.tselect,
        csr::TDATA1 => {
            let i = hart.csrs.tselect as usize;
            hart.csrs.tdata1[i]
        }
        csr::TDATA2 => {
            let i = hart.csrs.tselect as usize;
            hart.csrs.tdata2[i]
        }
        csr::TINFO => csr::TINFO_MCONTROL,
        csr::TCONTROL => hart.csrs.tcontrol & csr::TCONTROL_WRITABLE,
        _ => 0, // TDATA3 among them: not implemented
    }
}

impl CoreCtx<'_> {
    /// Returns `true` if the given CSR address corresponds to a CSR that is
    /// implemented by this hart.
    #[inline]
    pub const fn is_valid_csr(&self, addr: CsrAddr) -> bool {
        self.hart.is_valid_csr(addr)
    }

    /// Reads a value from a Control and Status Register (CSR).
    pub fn csr_read(&self, addr: CsrAddr) -> u64 {
        read(self.hart, self.uncore, addr)
    }

    /// Writes a value to a Control and Status Register (CSR).
    pub fn csr_write(&mut self, addr: CsrAddr, val: u64) {
        match addr {
            csr::FFLAGS => {
                self.hart.csrs.fflags = FpFlags::from_bits(val as u8);
                self.hart.csrs.mark_fs_dirty();
            }
            csr::FRM => {
                self.hart.csrs.frm = Frm::from_bits(val);
                self.hart.csrs.mark_fs_dirty();
            }
            csr::FCSR => {
                self.hart.csrs.set_fcsr(val);
                self.hart.csrs.mark_fs_dirty();
            }
            csr::CSR_SIM_PANIC => {
                self.trap(&Trap::RequestedTrap(val), self.hart.pc);
            }
            csr::MSTATUS => {
                // WARL: only defined writable bits are accepted; WPRI/SD/UXL/SXL ignored.
                const MSTATUS_WRITABLE: u64 = csr::MSTATUS_SIE
                    | csr::MSTATUS_MIE
                    | csr::MSTATUS_SPIE
                    | csr::MSTATUS_MPIE
                    | csr::MSTATUS_SPP
                    | csr::MSTATUS_MPP
                    | csr::MSTATUS_VS
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
                let mpp =
                    (self.hart.csrs.mstatus >> csr::MSTATUS_MPP_SHIFT) & csr::MSTATUS_MPP_MASK;
                if mpp == 2 {
                    self.hart.csrs.mstatus &= !csr::MSTATUS_MPP;
                }
            }
            csr::MEDELEG => {
                // An ecall from M-mode cannot be delegated.
                self.hart.csrs.medeleg = val & !(1 << exception::ENVIRONMENT_CALL_FROM_M_MODE);
            }
            csr::MIDELEG => {
                // Only S-level interrupts can be delegated (not M-level)
                let mask = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
                self.hart.csrs.mideleg = val & mask;
            }
            csr::MIE => {
                // WARL: only defined interrupt-enable bits are writable
                let mask = csr::MIE_SSIP
                    | csr::MIE_MSIP
                    | csr::MIE_STIE
                    | csr::MIE_MTIE
                    | csr::MIE_SEIP
                    | csr::MIE_MEIP;
                self.hart.csrs.mie = val & mask;
            }
            csr::MTVEC => self.hart.csrs.mtvec = legalize_tvec(val),
            csr::MISA => {
                // MISA is WARL: writes are silently ignored (extensions are hardwired).
            }
            csr::MSCRATCH => self.hart.csrs.mscratch = val,
            csr::MEPC => self.hart.csrs.mepc = self.hart.csrs.legal_epc(val),
            csr::MCAUSE => self.hart.csrs.mcause = val,
            csr::MTVAL => self.hart.csrs.mtval = val,
            csr::MIP => {
                let mask = csr::MIP_SSIP | csr::MIP_STIP | csr::MIP_SEIP;
                self.hart.csrs.mip = (self.hart.csrs.mip & !mask) | (val & mask);
                // Track software-written SEIP so pre_tick preserves it
                self.hart.sw_seip = (val & csr::MIP_SEIP) != 0;
            }
            csr::SSTATUS => {
                self.hart.csrs.mstatus = (self.hart.csrs.mstatus & !csr::SSTATUS_WRITABLE)
                    | (val & csr::SSTATUS_WRITABLE);
            }
            csr::SIE => {
                let mask = self.hart.csrs.mideleg;
                self.hart.csrs.mie = (self.hart.csrs.mie & !mask) | (val & mask);
            }
            csr::STVEC => self.hart.csrs.stvec = legalize_tvec(val),
            csr::SSCRATCH => self.hart.csrs.sscratch = val,
            csr::SEPC => self.hart.csrs.sepc = self.hart.csrs.legal_epc(val),
            csr::SCAUSE => self.hart.csrs.scause = val,
            csr::STVAL => self.hart.csrs.stval = val,
            csr::SIP => {
                let mask = self.hart.csrs.mideleg & (csr::MIP_SSIP);
                self.hart.csrs.mip = (self.hart.csrs.mip & !mask) | (val & mask);
            }
            csr::MCOUNTEREN => self.hart.csrs.mcounteren = val & csr::COUNTEREN_WRITABLE,
            csr::SCOUNTEREN => self.hart.csrs.scounteren = val & csr::COUNTEREN_WRITABLE,
            csr::MENVCFG => {
                let svadu = if self.config.isa.svadu { csr::MENVCFG_ADUE } else { 0 };
                let writable = csr::MENVCFG_WRITABLE | svadu;
                self.hart.csrs.menvcfg = legalize_envcfg(val, writable);
            }
            csr::SENVCFG => {
                self.hart.csrs.senvcfg = legalize_envcfg(val, csr::SENVCFG_WRITABLE);
            }
            csr::MCYCLE => self.hart.csrs.mcycle = val,
            csr::MINSTRET => self.hart.csrs.minstret = val,
            csr::MCOUNTINHIBIT => {
                self.hart.csrs.mcountinhibit = val & csr::MCOUNTINHIBIT_WRITABLE;
            }
            csr::PMPCFG0 => self.hart.pmp.set_cfg_word(PmpCfgCsr::Pmpcfg0, val),
            csr::PMPCFG2 => self.hart.pmp.set_cfg_word(PmpCfgCsr::Pmpcfg2, val),
            _ if let Some(index) = PmpIdx::of_pmpaddr(addr) => {
                self.hart.pmp.set_addr(index, val);
            }
            csr::STIMECMP => {
                self.hart.csrs.stimecmp = val;
                self.hart.csrs.mip &= !csr::MIP_STIP;
            }
            csr::SATP => {
                let supported = Satp::new(val)
                    .paging_mode()
                    .is_some_and(|m| m.is_at_most(self.core.mmu.paging_mode_max));
                // A write naming an unsupported MODE has no effect at all.
                // TLB entries are ASID-tagged: software orders a satp change
                // against cached translations with SFENCE.VMA.
                if supported {
                    self.hart.csrs.satp = val;
                }
            }
            // Writable vector CSRs
            csr::VSTART => self.hart.csrs.vstart = val,
            csr::VXSAT => self.hart.csrs.vxsat = val & csr::VXSAT_BIT,
            csr::VXRM => self.hart.csrs.vxrm = Vxrm::from_bits(val as u8),
            csr::VCSR => self.hart.csrs.set_vcsr(val),
            // VL, VTYPE, VLENB are read-only (writes silently ignored)
            // Sdtrig — trigger CSR writes
            csr::TSELECT => {
                self.hart.csrs.tselect = val.min(trigger::TRIGGER_COUNT as u64 - 1);
            }
            csr::TDATA1 => {
                let i = self.hart.csrs.tselect as usize;
                self.hart.csrs.tdata1[i] = trigger::tdata1_written(val);
            }
            csr::TDATA2 => {
                let i = self.hart.csrs.tselect as usize;
                self.hart.csrs.tdata2[i] = val;
            }
            csr::TCONTROL => self.hart.csrs.tcontrol = val & csr::TCONTROL_WRITABLE,
            _ => {} // TDATA3 (not implemented) and TINFO (read-only) among them
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    use crate::isa::csr;

    #[test]
    fn test_cpu_csr_read_write_mstatus() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        state.csr_write(csr::MSTATUS, 0xFFFF_FFFF_FFFF_FFFF);

        let mstatus = state.csr_read(csr::MSTATUS);
        assert_ne!(mstatus, 0xFFFF_FFFF_FFFF_FFFF);

        let sstatus = state.csr_read(csr::SSTATUS);
        assert_eq!(sstatus, mstatus & (csr::MSTATUS_SD | csr::SSTATUS_VISIBLE));
    }

    #[test]
    fn mstatus_reads_sd_when_only_the_vector_state_is_dirty() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        state.csr_write(csr::MSTATUS, csr::MSTATUS_VS_DIRTY);

        assert_ne!(state.csr_read(csr::MSTATUS) & csr::MSTATUS_SD, 0);
    }

    #[test]
    fn test_cpu_csr_read_write_fcsr() {
        let config = Config::minimal();
        let mut sys = crate::system::SystemState::build(&config);
        let mut state = sys.core_ctx(0);

        state.csr_write(csr::FCSR, 0xFF);
        assert_eq!(state.csr_read(csr::FCSR), 0xFF);
        assert_eq!(state.csr_read(csr::FFLAGS), 0x1F);
        assert_eq!(state.csr_read(csr::FRM), 0x7);
    }
}
