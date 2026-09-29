//! Control and Status Register (CSR) definitions and operations.

mod access;

use crate::isa::csr::{
    CBIE_FLUSH, CBIE_ILLEGAL, CBIE_INVAL, CBIE_RESERVED, CYCLE, CsrAddr, ENVCFG_CBIE, FCSR, FFLAGS,
    FRM, INSTRET, MCAUSE, MCOUNTEREN, MCOUNTINHIBIT, MCOUNTINHIBIT_CY, MCOUNTINHIBIT_IR,
    MCOUNTINHIBIT_WRITABLE, MCYCLE, MEDELEG, MENVCFG, MENVCFG_CBCFE, MENVCFG_CBIE_MASK,
    MENVCFG_CBIE_SHIFT, MENVCFG_CBZE, MEPC, MIDELEG, MIE, MINSTRET, MIP, MISA, MISA_EXT_C,
    MSCRATCH, MSTATUS, MSTATUS_FS, MSTATUS_FS_DIRTY, MSTATUS_SD, MSTATUS_VS, MSTATUS_VS_DIRTY,
    MTVAL, MTVEC, SATP, SATP_MODE_BARE, SATP_MODE_MASK, SATP_MODE_SHIFT, SCAUSE, SCOUNTEREN,
    SENVCFG, SENVCFG_CBCFE, SENVCFG_CBZE, SEPC, SIE, SIP, SSCRATCH, SSTATUS, SSTATUS_VISIBLE,
    SSTATUS_WRITABLE, STVAL, STVEC, VCSR, VL, VLENB, VSTART, VTYPE, VXRM, VXSAT,
};
use crate::isa::privileged::PagingMode;

/// The value a write of `val` leaves in an envcfg register whose
/// implemented fields are `writable`: every other field reads as zero, and
/// the reserved CBIE encoding is written as `0b00`, as spike does.
#[must_use]
pub const fn legalize_envcfg(val: u64, writable: u64) -> u64 {
    let val = val & writable;
    if (val >> MENVCFG_CBIE_SHIFT) & MENVCFG_CBIE_MASK == CBIE_RESERVED {
        return val & !ENVCFG_CBIE;
    }
    val
}

/// Outcome of resolving the CBIE field at the current privilege level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CboInvalAction {
    /// `cbo.inval` is illegal — raise an illegal-instruction trap.
    Illegal,
    /// Treat `cbo.inval` as `cbo.flush` (writeback then invalidate).
    Flush,
    /// Full invalidate; dirty data may be discarded.
    Invalidate,
}

/// Returns true when `cbo.zero` is permitted at the current privilege level
/// per the Zicboz spec: M-mode is always allowed, S-mode requires
/// `menvcfg.CBZE`, U-mode additionally requires `senvcfg.CBZE`.
pub const fn cboz_allowed(
    menvcfg: u64,
    senvcfg: u64,
    privilege: crate::isa::privileged::PrivilegeMode,
) -> bool {
    use crate::isa::privileged::PrivilegeMode;
    match privilege {
        PrivilegeMode::Machine => true,
        PrivilegeMode::Supervisor => (menvcfg & MENVCFG_CBZE) != 0,
        PrivilegeMode::User => (menvcfg & MENVCFG_CBZE) != 0 && (senvcfg & SENVCFG_CBZE) != 0,
    }
}

/// Returns true when `cbo.clean` and `cbo.flush` are permitted at the current
/// privilege level (gated by `menvcfg.CBCFE` in S/U; `senvcfg.CBCFE` further
/// gates U-mode). M-mode is always allowed.
pub const fn cbocf_allowed(
    menvcfg: u64,
    senvcfg: u64,
    privilege: crate::isa::privileged::PrivilegeMode,
) -> bool {
    use crate::isa::privileged::PrivilegeMode;
    match privilege {
        PrivilegeMode::Machine => true,
        PrivilegeMode::Supervisor => (menvcfg & MENVCFG_CBCFE) != 0,
        PrivilegeMode::User => (menvcfg & MENVCFG_CBCFE) != 0 && (senvcfg & SENVCFG_CBCFE) != 0,
    }
}

/// Resolves the effective `cbo.inval` action at the current privilege level.
///
/// M-mode is always full invalidate. S-mode reads menvcfg.CBIE alone. U-mode
/// takes the most-restrictive of the two CBIE fields (00 beats 01 beats 11).
pub const fn cbo_inval_action(
    menvcfg: u64,
    senvcfg: u64,
    privilege: crate::isa::privileged::PrivilegeMode,
) -> CboInvalAction {
    use crate::isa::privileged::PrivilegeMode;
    let m_field = (menvcfg >> MENVCFG_CBIE_SHIFT) & MENVCFG_CBIE_MASK;
    let s_field = (senvcfg >> MENVCFG_CBIE_SHIFT) & MENVCFG_CBIE_MASK;
    let effective = match privilege {
        PrivilegeMode::Machine => CBIE_INVAL,
        PrivilegeMode::Supervisor => m_field,
        PrivilegeMode::User => cbie_intersect(m_field, s_field),
    };
    match effective {
        CBIE_FLUSH => CboInvalAction::Flush,
        CBIE_INVAL => CboInvalAction::Invalidate,
        // CBIE_ILLEGAL (0b00) and the reserved 0b10 encoding both trap.
        _ => CboInvalAction::Illegal,
    }
}

/// Combines two CBIE field values via the spec's most-restrictive rule:
/// 00 dominates 01 dominates 11. The 10 encoding is reserved and treated
/// as illegal.
const fn cbie_intersect(a: u64, b: u64) -> u64 {
    if a == CBIE_ILLEGAL || b == CBIE_ILLEGAL {
        CBIE_ILLEGAL
    } else if a == CBIE_FLUSH || b == CBIE_FLUSH {
        CBIE_FLUSH
    } else if a == CBIE_INVAL && b == CBIE_INVAL {
        CBIE_INVAL
    } else {
        CBIE_ILLEGAL
    }
}

// Sdtrig CSRs are read-zero / write-ignored stubs so software that probes for
// trigger support doesn't take an illegal-instruction trap. Actual hardware
// trigger functionality is not implemented.

/// `status` as `mstatus` or `sstatus` reads: SD set exactly when FS or VS
/// is Dirty.
#[must_use]
pub const fn with_state_dirty(status: u64) -> u64 {
    let val = status & !MSTATUS_SD;
    let fs_dirty = val & MSTATUS_FS == MSTATUS_FS_DIRTY;
    let vs_dirty = val & MSTATUS_VS == MSTATUS_VS_DIRTY;
    if fs_dirty || vs_dirty { val | MSTATUS_SD } else { val }
}

/// The low PC bits IALIGN keeps clear: bit 0 with the C extension
/// (IALIGN=16), bits 1:0 without it (IALIGN=32).
#[must_use]
pub const fn ialign_low_bits(misa: u64) -> u64 {
    if misa & MISA_EXT_C != 0 { 0b01 } else { 0b11 }
}

/// CSR serialization requirement classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsrSerializationType {
    /// CSR requires full memory fence and cache/TLB invalidation
    FenceRequired,
    /// CSR requires pipeline drain before execution
    Serializing,
    /// CSR has no special serialization requirements
    Relaxed,
}

/// Returns the serialization requirement for a given CSR address.
pub const fn csr_serialization_type(addr: CsrAddr) -> CsrSerializationType {
    match addr.as_u32() {
        // Fence-requiring CSRs
        x if x == SATP.as_u32() => CsrSerializationType::FenceRequired,

        // Serializing CSRs
        x if x == MSTATUS.as_u32()
            || x == SSTATUS.as_u32()
            || x == MTVEC.as_u32()
            || x == STVEC.as_u32()
            || x == MEDELEG.as_u32()
            || x == MIDELEG.as_u32()
            || x == VSTART.as_u32()
            || x == VXRM.as_u32() =>
        {
            CsrSerializationType::Serializing
        }

        // Relaxed CSRs (default)
        _ => CsrSerializationType::Relaxed,
    }
}

/// Control and Status Register file.
///
/// Contains all machine-level and supervisor-level CSRs that control processor state,
/// interrupt handling, memory management, and performance counters.
#[derive(Clone, Default, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Csrs {
    /// Machine status register.
    pub mstatus: u64,
    /// Machine ISA register.
    pub misa: u64,
    /// Machine exception delegation.
    pub medeleg: u64,
    /// Machine interrupt delegation.
    pub mideleg: u64,
    /// Machine interrupt enable.
    pub mie: u64,
    /// Machine trap vector base address.
    pub mtvec: u64,
    /// Machine scratch register.
    pub mscratch: u64,
    /// Machine exception program counter.
    pub mepc: u64,
    /// Machine trap cause.
    pub mcause: u64,
    /// Machine trap value.
    pub mtval: u64,
    /// Machine interrupt pending.
    pub mip: u64,
    /// Supervisor interrupt enable (masked view of `mie`).
    pub sie: u64,
    /// Supervisor trap vector base address.
    pub stvec: u64,
    /// Supervisor scratch register.
    pub sscratch: u64,
    /// Supervisor exception program counter.
    pub sepc: u64,
    /// Supervisor trap cause.
    pub scause: u64,
    /// Supervisor trap value.
    pub stval: u64,
    /// Supervisor interrupt pending (masked view of `mip`).
    pub sip: u64,
    /// Supervisor address translation and protection (SATP).
    pub satp: u64,
    /// Machine cycle counter: counts this hart's clock cycles unless
    /// `mcountinhibit.CY` is set; `cycle` is its read-only alias.
    pub mcycle: u64,
    /// Machine instructions-retired counter: counts every retired
    /// instruction unless `mcountinhibit.IR` is set; `instret` is its
    /// read-only alias.
    pub minstret: u64,
    /// Machine counter-inhibit register (only the CY and IR bits exist).
    pub mcountinhibit: u64,
    /// Supervisor timer compare (for timer interrupt).
    pub stimecmp: u64,
    /// Floating-point accrued exception flags (5 bits: NV, DZ, OF, UF, NX).
    pub fflags: u64,
    /// Floating-point dynamic rounding mode (3 bits).
    pub frm: u64,
    /// Machine counter-enable register.
    pub mcounteren: u64,
    /// Supervisor counter-enable register.
    pub scounteren: u64,
    /// Machine environment configuration register.
    pub menvcfg: u64,
    /// Supervisor environment configuration register.
    pub senvcfg: u64,
    /// Vector start position.
    pub vstart: u64,
    /// Vector fixed-point saturation flag.
    pub vxsat: u64,
    /// Vector fixed-point rounding mode.
    pub vxrm: u64,
    /// Vector length (set by vsetvl family only).
    pub vl: u64,
    /// Vector type (set by vsetvl family only).
    pub vtype: u64,
    /// Vector register byte length (VLEN/8, constant).
    pub vlenb: u64,
    /// Currently selected trigger index (tselect).
    pub tselect: u64,
    /// Trigger data1 per slot (mcontrol config).
    pub tdata1: [u64; 2],
    /// Trigger data2 per slot (address match value).
    pub tdata2: [u64; 2],
    /// Trigger control register (mte=bit3, mpte=bit7).
    pub tcontrol: u64,
}

impl Csrs {
    /// The vector configuration `vtype`, `vl` and `vstart` hold.
    #[must_use]
    pub const fn vector_config(&self) -> crate::isa::rvv::VectorConfig {
        crate::isa::rvv::VectorConfig { vtype: self.vtype, vl: self.vl, vstart: self.vstart }
    }

    /// Advances `mcycle` by one clock unless inhibited.
    pub const fn count_cycle(&mut self) {
        if self.mcountinhibit & MCOUNTINHIBIT_CY == 0 {
            self.mcycle = self.mcycle.wrapping_add(1);
        }
    }

    /// Counts `cycles` cycles in `mcycle` unless inhibited.
    pub const fn count_cycles(&mut self, cycles: u64) {
        if self.mcountinhibit & MCOUNTINHIBIT_CY == 0 {
            self.mcycle = self.mcycle.wrapping_add(cycles);
        }
    }

    /// Counts one retired instruction in `minstret` unless inhibited.
    pub const fn count_retired(&mut self) {
        if self.mcountinhibit & MCOUNTINHIBIT_IR == 0 {
            self.minstret = self.minstret.wrapping_add(1);
        }
    }

    /// `sstatus`: the fields of `mstatus` supervisor mode sees, without SD.
    #[must_use]
    pub const fn sstatus(&self) -> u64 {
        self.mstatus & SSTATUS_VISIBLE
    }

    /// Reads a CSR value by its address. Returns 0 for unrecognized addresses.
    pub const fn read(&self, addr: CsrAddr) -> u64 {
        match addr.as_u32() {
            x if x == FFLAGS.as_u32() => self.fflags & 0x1F,
            x if x == FRM.as_u32() => self.frm & 0x7,
            x if x == FCSR.as_u32() => ((self.frm & 0x7) << 5) | (self.fflags & 0x1F),
            x if x == MSTATUS.as_u32() => with_state_dirty(self.mstatus),
            x if x == MISA.as_u32() => self.misa,
            x if x == MEDELEG.as_u32() => self.medeleg,
            x if x == MIDELEG.as_u32() => self.mideleg,
            x if x == MIE.as_u32() => self.mie,
            x if x == MTVEC.as_u32() => self.mtvec,
            x if x == MSCRATCH.as_u32() => self.mscratch,
            x if x == MEPC.as_u32() => self.mepc,
            x if x == MCAUSE.as_u32() => self.mcause,
            x if x == MTVAL.as_u32() => self.mtval,
            x if x == MIP.as_u32() => self.mip,
            x if x == SSTATUS.as_u32() => with_state_dirty(self.sstatus()),
            x if x == SIE.as_u32() => self.sie,
            x if x == STVEC.as_u32() => self.stvec,
            x if x == SSCRATCH.as_u32() => self.sscratch,
            x if x == SEPC.as_u32() => self.sepc,
            x if x == SCAUSE.as_u32() => self.scause,
            x if x == STVAL.as_u32() => self.stval,
            x if x == SIP.as_u32() => self.sip,
            x if x == SATP.as_u32() => self.satp,
            x if x == CYCLE.as_u32() || x == MCYCLE.as_u32() => self.mcycle,
            x if x == INSTRET.as_u32() || x == MINSTRET.as_u32() => self.minstret,
            x if x == MCOUNTINHIBIT.as_u32() => self.mcountinhibit,
            x if x == MCOUNTEREN.as_u32() => self.mcounteren,
            x if x == SCOUNTEREN.as_u32() => self.scounteren,
            x if x == MENVCFG.as_u32() => self.menvcfg,
            x if x == SENVCFG.as_u32() => self.senvcfg,
            x if x == VSTART.as_u32() => self.vstart,
            x if x == VXSAT.as_u32() => self.vxsat & 0x1,
            x if x == VXRM.as_u32() => self.vxrm & 0x3,
            x if x == VCSR.as_u32() => (self.vxsat & 0x1) | ((self.vxrm & 0x3) << 1),
            x if x == VL.as_u32() => self.vl,
            x if x == VTYPE.as_u32() => self.vtype,
            x if x == VLENB.as_u32() => self.vlenb,
            _ => 0,
        }
    }

    /// Writes a value to a CSR by its address.
    pub const fn write(&mut self, addr: CsrAddr, val: u64) {
        match addr.as_u32() {
            x if x == FFLAGS.as_u32() => self.fflags = val & 0x1F,
            x if x == FRM.as_u32() => self.frm = val & 0x7,
            x if x == FCSR.as_u32() => {
                self.fflags = val & 0x1F;
                self.frm = (val >> 5) & 0x7;
            }
            x if x == MSTATUS.as_u32() => self.mstatus = val,
            x if x == MISA.as_u32() => self.misa = val,
            x if x == MEDELEG.as_u32() => self.medeleg = val,
            x if x == MIDELEG.as_u32() => self.mideleg = val,
            x if x == MIE.as_u32() => self.mie = val,
            x if x == MTVEC.as_u32() => self.mtvec = val,
            x if x == MSCRATCH.as_u32() => self.mscratch = val,
            x if x == MEPC.as_u32() => self.mepc = val,
            x if x == MCAUSE.as_u32() => self.mcause = val,
            x if x == MTVAL.as_u32() => self.mtval = val,
            x if x == MIP.as_u32() => self.mip = val,
            x if x == SSTATUS.as_u32() => {
                self.mstatus = (self.mstatus & !SSTATUS_WRITABLE) | (val & SSTATUS_WRITABLE);
            }
            x if x == SIE.as_u32() => self.sie = val,
            x if x == STVEC.as_u32() => self.stvec = val,
            x if x == SSCRATCH.as_u32() => self.sscratch = val,
            x if x == SEPC.as_u32() => self.sepc = val,
            x if x == SCAUSE.as_u32() => self.scause = val,
            x if x == STVAL.as_u32() => self.stval = val,
            x if x == SIP.as_u32() => self.sip = val,
            x if x == SATP.as_u32() => {
                let mode = (val >> SATP_MODE_SHIFT) & SATP_MODE_MASK;
                let new_mode =
                    if PagingMode::from_satp_mode(mode).is_some() { mode } else { SATP_MODE_BARE };
                let mask = !(SATP_MODE_MASK << SATP_MODE_SHIFT);
                self.satp = (val & mask) | (new_mode << SATP_MODE_SHIFT);
            }
            x if x == MCYCLE.as_u32() => self.mcycle = val,
            x if x == MINSTRET.as_u32() => self.minstret = val,
            x if x == MCOUNTINHIBIT.as_u32() => self.mcountinhibit = val & MCOUNTINHIBIT_WRITABLE,
            x if x == MCOUNTEREN.as_u32() => self.mcounteren = val,
            x if x == SCOUNTEREN.as_u32() => self.scounteren = val,
            x if x == MENVCFG.as_u32() => self.menvcfg = val,
            x if x == SENVCFG.as_u32() => self.senvcfg = val,
            x if x == VSTART.as_u32() => self.vstart = val,
            x if x == VXSAT.as_u32() => self.vxsat = val & 0x1,
            x if x == VXRM.as_u32() => self.vxrm = val & 0x3,
            x if x == VCSR.as_u32() => {
                self.vxsat = val & 0x1;
                self.vxrm = (val >> 1) & 0x3;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isa::csr::{
        MSTATUS_MIE, MSTATUS_MPP, MSTATUS_MXR, MSTATUS_SIE, MSTATUS_SUM, SATP_MODE_SV39,
    };

    #[test]
    fn an_sstatus_write_lands_in_mstatus() {
        let mut csrs = Csrs::default();

        csrs.write(SSTATUS, MSTATUS_SIE | MSTATUS_SUM | MSTATUS_MIE);

        assert_eq!(
            csrs.read(MSTATUS) & (MSTATUS_SIE | MSTATUS_SUM | MSTATUS_MIE),
            MSTATUS_SIE | MSTATUS_SUM
        );
    }

    #[test]
    fn an_mstatus_write_shows_through_sstatus() {
        let mut csrs = Csrs::default();

        csrs.write(MSTATUS, MSTATUS_MXR | MSTATUS_MPP);

        assert_eq!(csrs.read(SSTATUS), MSTATUS_MXR);
    }

    #[test]
    fn test_csr_serialization_type() {
        assert_eq!(csr_serialization_type(SATP), CsrSerializationType::FenceRequired);
        assert_eq!(csr_serialization_type(MSTATUS), CsrSerializationType::Serializing);
        assert_eq!(csr_serialization_type(SSTATUS), CsrSerializationType::Serializing);
        assert_eq!(csr_serialization_type(MTVEC), CsrSerializationType::Serializing);
        assert_eq!(csr_serialization_type(STVEC), CsrSerializationType::Serializing);
        assert_eq!(csr_serialization_type(MEDELEG), CsrSerializationType::Serializing);
        assert_eq!(csr_serialization_type(MIDELEG), CsrSerializationType::Serializing);
        assert_eq!(csr_serialization_type(MCAUSE), CsrSerializationType::Relaxed);
        assert_eq!(csr_serialization_type(CsrAddr::from_u32(0)), CsrSerializationType::Relaxed);
    }

    #[test]
    fn test_csrs_read_write() {
        let mut csrs = Csrs::default();

        csrs.write(MSTATUS, 0x1234);
        assert_eq!(csrs.read(MSTATUS), 0x1234);

        csrs.write(MISA, 0x5678);
        assert_eq!(csrs.read(MISA), 0x5678);

        csrs.write(SATP, SATP_MODE_SV39 << SATP_MODE_SHIFT | 0xabc);
        assert_eq!(csrs.read(SATP), SATP_MODE_SV39 << SATP_MODE_SHIFT | 0xabc);

        csrs.write(SATP, 0xF << SATP_MODE_SHIFT | 0xdef);
        assert_eq!(csrs.read(SATP), SATP_MODE_BARE << SATP_MODE_SHIFT | 0xdef);

        csrs.write(FFLAGS, 0x1F);
        assert_eq!(csrs.read(FFLAGS), 0x1F);

        csrs.write(FRM, 0x7);
        assert_eq!(csrs.read(FRM), 0x7);
        assert_eq!(csrs.read(FCSR), (0x7 << 5) | 0x1F);

        csrs.write(FCSR, (0x3 << 5) | 0xA);
        assert_eq!(csrs.read(FRM), 0x3);
        assert_eq!(csrs.read(FFLAGS), 0xA);

        csrs.write(CsrAddr::from_u32(9999), 0x1);
        assert_eq!(csrs.read(CsrAddr::from_u32(9999)), 0x0);
    }
}
