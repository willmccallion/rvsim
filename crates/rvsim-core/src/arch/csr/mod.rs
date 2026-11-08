//! Control and Status Register (CSR) definitions and operations.

mod access;

use crate::isa::csr::{
    CBIE_FLUSH, CBIE_ILLEGAL, CBIE_RESERVED, CYCLE, CsrAddr, ENVCFG_CBIE, FCSR, FFLAGS, FRM,
    INSTRET, MCAUSE, MCOUNTEREN, MCOUNTINHIBIT, MCOUNTINHIBIT_CY, MCOUNTINHIBIT_IR,
    MCOUNTINHIBIT_WRITABLE, MCYCLE, MEDELEG, MENVCFG, MENVCFG_CBCFE, MENVCFG_CBIE_MASK,
    MENVCFG_CBIE_SHIFT, MENVCFG_CBZE, MEPC, MIDELEG, MIE, MINSTRET, MIP, MISA, MSCRATCH, MSTATUS,
    MSTATUS_FS, MSTATUS_FS_DIRTY, MSTATUS_SD, MSTATUS_VS, MSTATUS_VS_DIRTY, MTVAL, MTVEC, SATP,
    SATP_MODE_BARE, SATP_MODE_MASK, SATP_MODE_SHIFT, SCAUSE, SCOUNTEREN, SENVCFG, SENVCFG_CBCFE,
    SENVCFG_CBZE, SEPC, SIE, SIP, SSCRATCH, SSTATUS, SSTATUS_VISIBLE, SSTATUS_WRITABLE, STVAL,
    STVEC, VCSR, VL, VLENB, VSTART, VTYPE, VXRM, VXSAT, VXSAT_BIT,
};
use crate::isa::fp::{FpFlags, Frm};
use crate::isa::misa::Misa;
use crate::isa::privileged::{PagingMode, Satp};
use crate::isa::rvv::Vxrm;

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
    let effective = match privilege {
        PrivilegeMode::Machine => Cbie::Invalidate,
        PrivilegeMode::Supervisor => Cbie::of(menvcfg),
        PrivilegeMode::User => Cbie::of(menvcfg).intersect(Cbie::of(senvcfg)),
    };
    match effective {
        Cbie::Flush => CboInvalAction::Flush,
        Cbie::Invalidate => CboInvalAction::Invalidate,
        Cbie::Illegal | Cbie::Reserved => CboInvalAction::Illegal,
    }
}

/// An envcfg register's CBIE field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cbie {
    /// `0b00`: `cbo.inval` is illegal.
    Illegal,
    /// `0b01`: `cbo.inval` flushes.
    Flush,
    /// `0b10`: reserved, which a debugger's raw write can still store.
    Reserved,
    /// `0b11`: `cbo.inval` invalidates.
    Invalidate,
}

impl Cbie {
    /// The CBIE field of `envcfg`.
    const fn of(envcfg: u64) -> Self {
        match (envcfg >> MENVCFG_CBIE_SHIFT) & MENVCFG_CBIE_MASK {
            CBIE_ILLEGAL => Self::Illegal,
            CBIE_FLUSH => Self::Flush,
            CBIE_RESERVED => Self::Reserved,
            _ => Self::Invalidate,
        }
    }

    /// The spec's most-restrictive combination of two fields: Illegal
    /// dominates Flush dominates Invalidate, and Reserved acts as Illegal
    /// except beside a Flush.
    const fn intersect(self, other: Self) -> Self {
        match (self, other) {
            (Self::Illegal, _) | (_, Self::Illegal) => Self::Illegal,
            (Self::Flush, _) | (_, Self::Flush) => Self::Flush,
            (Self::Invalidate, Self::Invalidate) => Self::Invalidate,
            _ => Self::Illegal,
        }
    }
}

/// `status` as `mstatus` or `sstatus` reads: SD set exactly when FS or VS
/// is Dirty.
#[must_use]
pub const fn with_state_dirty(status: u64) -> u64 {
    let val = status & !MSTATUS_SD;
    let fs_dirty = val & MSTATUS_FS == MSTATUS_FS_DIRTY;
    let vs_dirty = val & MSTATUS_VS == MSTATUS_VS_DIRTY;
    if fs_dirty || vs_dirty { val | MSTATUS_SD } else { val }
}

/// `mtvec`/`stvec` as a write leaves them: the reserved modes 2 and 3 fall
/// back to Direct.
#[must_use]
pub const fn legalize_tvec(val: u64) -> u64 {
    if val & 3 >= 2 { val & !3 } else { val }
}

/// The low PC bits IALIGN keeps clear: bit 0 with the C extension
/// (IALIGN=16), bits 1:0 without it (IALIGN=32).
#[must_use]
pub const fn ialign_low_bits(misa: Misa) -> u64 {
    if misa.has_c() { 0b01 } else { 0b11 }
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
    match addr {
        // Fence-requiring CSRs
        SATP => CsrSerializationType::FenceRequired,

        // Serializing CSRs
        MSTATUS | SSTATUS | MTVEC | STVEC | MEDELEG | MIDELEG | VSTART | VXRM => {
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
    pub misa: Misa,
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
    /// Floating-point accrued exception flags.
    pub fflags: FpFlags,
    /// Floating-point dynamic rounding mode.
    pub frm: Frm,
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
    pub vxrm: Vxrm,
    /// Vector length (set by vsetvl family only).
    pub vl: u64,
    /// Vector type (set by vsetvl family only).
    pub vtype: u64,
    /// Vector register byte length (VLEN/8, constant).
    pub vlenb: u64,
    /// Currently selected trigger index (tselect).
    pub tselect: u64,
    /// Trigger data1 per slot (mcontrol config).
    pub tdata1: [u64; crate::arch::trigger::TRIGGER_COUNT],
    /// Trigger data2 per slot (address match value).
    pub tdata2: [u64; crate::arch::trigger::TRIGGER_COUNT],
    /// Trigger control register (mte=bit3, mpte=bit7).
    pub tcontrol: u64,
}

impl Csrs {
    /// The vector configuration `vtype`, `vl` and `vstart` hold.
    #[must_use]
    pub const fn vector_config(&self) -> crate::isa::rvv::VectorConfig {
        crate::isa::rvv::VectorConfig::new(self.vtype, self.vl, self.vstart)
    }

    /// Marks the FP state dirty, as every write to it does.
    pub const fn mark_fs_dirty(&mut self) {
        self.mstatus = (self.mstatus & !MSTATUS_FS) | MSTATUS_FS_DIRTY;
    }

    /// `mepc`/`sepc` as a write of `val` leaves them: IALIGN's low bits clear.
    #[must_use]
    pub const fn legal_epc(&self, val: u64) -> u64 {
        val & !ialign_low_bits(self.misa)
    }

    /// `fflags` as the CSR reads.
    #[must_use]
    pub const fn fflags_bits(&self) -> u64 {
        self.fflags.bits() as u64
    }

    /// `fcsr`: `frm` above `fflags`.
    #[must_use]
    pub const fn fcsr(&self) -> u64 {
        (self.frm.bits() << 5) | self.fflags_bits()
    }

    /// Writes `fcsr`, which sets both `frm` and `fflags`.
    pub const fn set_fcsr(&mut self, val: u64) {
        self.fflags = FpFlags::from_csr(val);
        self.frm = Frm::from_bits(val >> 5);
    }

    /// `vxrm` as the CSR reads.
    #[must_use]
    pub const fn vxrm_bits(&self) -> u64 {
        self.vxrm.to_bits() as u64
    }

    /// `vcsr`: `vxrm` above `vxsat`.
    #[must_use]
    pub const fn vcsr(&self) -> u64 {
        (self.vxsat & VXSAT_BIT) | (self.vxrm_bits() << 1)
    }

    /// Writes `vcsr`, which sets both `vxrm` and `vxsat`.
    pub const fn set_vcsr(&mut self, val: u64) {
        self.vxsat = val & VXSAT_BIT;
        self.vxrm = Vxrm::from_csr(val >> 1);
    }

    /// The rounding modes `vxrm` and `frm` hold. A reserved `frm` reads as
    /// RNE: execute raises illegal-instruction for any vector FP
    /// instruction while `frm` is reserved, so nothing rounds by it.
    #[must_use]
    pub fn vector_rounding(&self) -> crate::isa::rvv::VectorRounding {
        crate::isa::rvv::VectorRounding {
            vxrm: self.vxrm,
            frm: self.frm.rounding_mode().unwrap_or_default(),
        }
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
        match addr {
            FFLAGS => self.fflags_bits(),
            FRM => self.frm.bits(),
            FCSR => self.fcsr(),
            MSTATUS => with_state_dirty(self.mstatus),
            MISA => self.misa.bits(),
            MEDELEG => self.medeleg,
            MIDELEG => self.mideleg,
            MIE => self.mie,
            MTVEC => self.mtvec,
            MSCRATCH => self.mscratch,
            MEPC => self.mepc,
            MCAUSE => self.mcause,
            MTVAL => self.mtval,
            MIP => self.mip,
            SSTATUS => with_state_dirty(self.sstatus()),
            SIE => self.sie,
            STVEC => self.stvec,
            SSCRATCH => self.sscratch,
            SEPC => self.sepc,
            SCAUSE => self.scause,
            STVAL => self.stval,
            SIP => self.sip,
            SATP => self.satp,
            CYCLE | MCYCLE => self.mcycle,
            INSTRET | MINSTRET => self.minstret,
            MCOUNTINHIBIT => self.mcountinhibit,
            MCOUNTEREN => self.mcounteren,
            SCOUNTEREN => self.scounteren,
            MENVCFG => self.menvcfg,
            SENVCFG => self.senvcfg,
            VSTART => self.vstart,
            VXSAT => self.vxsat & VXSAT_BIT,
            VXRM => self.vxrm_bits(),
            VCSR => self.vcsr(),
            VL => self.vl,
            VTYPE => self.vtype,
            VLENB => self.vlenb,
            _ => 0,
        }
    }

    /// Writes a value to a CSR by its address.
    pub const fn write(&mut self, addr: CsrAddr, val: u64) {
        match addr {
            FFLAGS => self.fflags = FpFlags::from_csr(val),
            FRM => self.frm = Frm::from_bits(val),
            FCSR => self.set_fcsr(val),
            MSTATUS => self.mstatus = val,
            MISA => self.misa = Misa::from_bits(val),
            MEDELEG => self.medeleg = val,
            MIDELEG => self.mideleg = val,
            MIE => self.mie = val,
            MTVEC => self.mtvec = val,
            MSCRATCH => self.mscratch = val,
            MEPC => self.mepc = val,
            MCAUSE => self.mcause = val,
            MTVAL => self.mtval = val,
            MIP => self.mip = val,
            SSTATUS => {
                self.mstatus = (self.mstatus & !SSTATUS_WRITABLE) | (val & SSTATUS_WRITABLE);
            }
            SIE => self.sie = val,
            STVEC => self.stvec = val,
            SSCRATCH => self.sscratch = val,
            SEPC => self.sepc = val,
            SCAUSE => self.scause = val,
            STVAL => self.stval = val,
            SIP => self.sip = val,
            SATP => {
                let mode = Satp::new(val).mode_field();
                let new_mode =
                    if PagingMode::from_satp_mode(mode).is_some() { mode } else { SATP_MODE_BARE };
                let mask = !(SATP_MODE_MASK << SATP_MODE_SHIFT);
                self.satp = (val & mask) | (new_mode << SATP_MODE_SHIFT);
            }
            MCYCLE => self.mcycle = val,
            MINSTRET => self.minstret = val,
            MCOUNTINHIBIT => self.mcountinhibit = val & MCOUNTINHIBIT_WRITABLE,
            MCOUNTEREN => self.mcounteren = val,
            SCOUNTEREN => self.scounteren = val,
            MENVCFG => self.menvcfg = val,
            SENVCFG => self.senvcfg = val,
            VSTART => self.vstart = val,
            VXSAT => self.vxsat = val & VXSAT_BIT,
            VXRM => self.vxrm = Vxrm::from_csr(val),
            VCSR => self.set_vcsr(val),
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
    fn satp_requires_a_fence_and_trap_setup_csrs_serialize() {
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
    fn csr_writes_read_back_with_satp_legalized_and_fcsr_aliased() {
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
