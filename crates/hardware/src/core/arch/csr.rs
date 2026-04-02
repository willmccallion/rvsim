//! Control and Status Register (CSR) definitions and operations.
//!
//! This module implements the CSR subsystem for the RISC-V processor. It provides:
//! 1. **Address Definitions:** Constants for all standard machine and supervisor CSRs.
//! 2. **Field Masks:** Bitmasks and shifts for status, ISA, and translation control.
//! 3. **Register Storage:** The `Csrs` struct for maintaining architectural state.
//! 4. **Access Logic:** Standardized read and write operations for register interaction.

use crate::common::CsrAddr;

/// Vector start position CSR address.
pub const VSTART: CsrAddr = CsrAddr::from_u32(0x008);

/// Vector fixed-point saturation flag CSR address.
pub const VXSAT: CsrAddr = CsrAddr::from_u32(0x009);

/// Vector fixed-point rounding mode CSR address.
pub const VXRM: CsrAddr = CsrAddr::from_u32(0x00A);

/// Vector control and status register (combined vxsat|vxrm) CSR address.
pub const VCSR: CsrAddr = CsrAddr::from_u32(0x00F);

/// Vector length CSR address (read-only, set by vsetvl family).
pub const VL: CsrAddr = CsrAddr::from_u32(0xC20);

/// Vector type CSR address (read-only, set by vsetvl family).
pub const VTYPE: CsrAddr = CsrAddr::from_u32(0xC21);

/// Vector register byte length CSR address (read-only, = VLEN/8).
pub const VLENB: CsrAddr = CsrAddr::from_u32(0xC22);

/// Floating-point accrued exceptions CSR address.
pub const FFLAGS: CsrAddr = CsrAddr::from_u32(0x001);

/// Floating-point dynamic rounding mode CSR address.
pub const FRM: CsrAddr = CsrAddr::from_u32(0x002);

/// Floating-point control and status register CSR address.
pub const FCSR: CsrAddr = CsrAddr::from_u32(0x003);

/// Machine vendor ID CSR address.
pub const MVENDORID: CsrAddr = CsrAddr::from_u32(0xF11);

/// Machine architecture ID CSR address.
pub const MARCHID: CsrAddr = CsrAddr::from_u32(0xF12);

/// Machine implementation ID CSR address.
pub const MIMPID: CsrAddr = CsrAddr::from_u32(0xF13);

/// Machine hardware thread ID CSR address.
pub const MHARTID: CsrAddr = CsrAddr::from_u32(0xF14);

/// Machine status register CSR address.
pub const MSTATUS: CsrAddr = CsrAddr::from_u32(0x300);

/// Machine ISA register CSR address.
pub const MISA: CsrAddr = CsrAddr::from_u32(0x301);

/// Machine exception delegation register CSR address.
pub const MEDELEG: CsrAddr = CsrAddr::from_u32(0x302);

/// Machine interrupt delegation register CSR address.
pub const MIDELEG: CsrAddr = CsrAddr::from_u32(0x303);

/// Machine interrupt enable register CSR address.
pub const MIE: CsrAddr = CsrAddr::from_u32(0x304);

/// Machine trap vector base address register CSR address.
pub const MTVEC: CsrAddr = CsrAddr::from_u32(0x305);

/// Machine counter enable register CSR address.
pub const MCOUNTEREN: CsrAddr = CsrAddr::from_u32(0x306);

/// Machine environment configuration register CSR address.
pub const MENVCFG: CsrAddr = CsrAddr::from_u32(0x30A);

/// STCE bit in menvcfg — enables Sstc (hardware stimecmp-based STIP) for S-mode.
pub const MENVCFG_STCE: u64 = 1 << 63;

/// Machine scratch register CSR address.
pub const MSCRATCH: CsrAddr = CsrAddr::from_u32(0x340);

/// Machine exception program counter CSR address.
pub const MEPC: CsrAddr = CsrAddr::from_u32(0x341);

/// Machine cause register CSR address.
pub const MCAUSE: CsrAddr = CsrAddr::from_u32(0x342);

/// Machine trap value register CSR address.
pub const MTVAL: CsrAddr = CsrAddr::from_u32(0x343);

/// Machine interrupt pending register CSR address.
pub const MIP: CsrAddr = CsrAddr::from_u32(0x344);

/// PMP configuration register 0 (entries 0–7) CSR address.
pub const PMPCFG0: CsrAddr = CsrAddr::from_u32(0x3A0);

/// PMP configuration register 2 (entries 8–15) CSR address.
/// Note: pmpcfg1 / pmpcfg3 do not exist in RV64.
pub const PMPCFG2: CsrAddr = CsrAddr::from_u32(0x3A2);

/// First PMP address register CSR address (pmpaddr0).
pub const PMPADDR0: CsrAddr = CsrAddr::from_u32(0x3B0);

/// Last PMP address register CSR address (pmpaddr15).
pub const PMPADDR15: CsrAddr = CsrAddr::from_u32(0x3BF);

/// Machine counter-inhibit register CSR address.
pub const MCOUNTINHIBIT: CsrAddr = CsrAddr::from_u32(0x320);

/// First machine hardware performance-monitoring event selector (mhpmevent3).
pub const MHPMEVENT3: CsrAddr = CsrAddr::from_u32(0x323);

/// Last machine hardware performance-monitoring event selector (mhpmevent31).
pub const MHPMEVENT31: CsrAddr = CsrAddr::from_u32(0x33F);

/// First machine hardware performance-monitoring counter (mhpmcounter3).
pub const MHPMCOUNTER3: CsrAddr = CsrAddr::from_u32(0xB03);

/// Last machine hardware performance-monitoring counter (mhpmcounter31).
pub const MHPMCOUNTER31: CsrAddr = CsrAddr::from_u32(0xB1F);

/// Supervisor status register CSR address.
pub const SSTATUS: CsrAddr = CsrAddr::from_u32(0x100);

/// Supervisor interrupt enable register CSR address.
pub const SIE: CsrAddr = CsrAddr::from_u32(0x104);

/// Supervisor trap vector base address register CSR address.
pub const STVEC: CsrAddr = CsrAddr::from_u32(0x105);

/// Supervisor counter enable register CSR address.
pub const SCOUNTEREN: CsrAddr = CsrAddr::from_u32(0x106);

/// Supervisor scratch register CSR address.
pub const SSCRATCH: CsrAddr = CsrAddr::from_u32(0x140);

/// Supervisor exception program counter CSR address.
pub const SEPC: CsrAddr = CsrAddr::from_u32(0x141);

/// Supervisor cause register CSR address.
pub const SCAUSE: CsrAddr = CsrAddr::from_u32(0x142);

/// Supervisor trap value register CSR address.
pub const STVAL: CsrAddr = CsrAddr::from_u32(0x143);

/// Supervisor interrupt pending register CSR address.
pub const SIP: CsrAddr = CsrAddr::from_u32(0x144);

/// Supervisor address translation and protection register CSR address.
pub const SATP: CsrAddr = CsrAddr::from_u32(0x180);

/// Supervisor timer compare register CSR address.
pub const STIMECMP: CsrAddr = CsrAddr::from_u32(0x14D);

/// Cycle counter CSR address (read-only, user mode accessible).
pub const CYCLE: CsrAddr = CsrAddr::from_u32(0xC00);

/// Real-time counter CSR address (read-only, user mode accessible).
pub const TIME: CsrAddr = CsrAddr::from_u32(0xC01);

/// Instructions retired counter CSR address (read-only, user mode accessible).
pub const INSTRET: CsrAddr = CsrAddr::from_u32(0xC02);

/// Machine cycle counter CSR address.
pub const MCYCLE: CsrAddr = CsrAddr::from_u32(0xB00);

/// Machine instructions retired counter CSR address.
pub const MINSTRET: CsrAddr = CsrAddr::from_u32(0xB02);

/// User interrupt enable bit in `mstatus` register.
pub const MSTATUS_UIE: u64 = 1 << 0;

/// Supervisor interrupt enable bit in `mstatus` register.
pub const MSTATUS_SIE: u64 = 1 << 1;

/// Machine interrupt enable bit in `mstatus` register.
pub const MSTATUS_MIE: u64 = 1 << 3;

/// User software interrupt enable bit in `mie` register.
pub const MIE_USIP: u64 = 1 << 0;

/// Supervisor software interrupt enable bit in `mie` register.
pub const MIE_SSIP: u64 = 1 << 1;

/// Machine software interrupt enable bit in `mie` register.
pub const MIE_MSIP: u64 = 1 << 3;

/// User timer interrupt enable bit in `mie` register.
pub const MIE_UTIE: u64 = 1 << 4;

/// Supervisor timer interrupt enable bit in `mie` register.
pub const MIE_STIE: u64 = 1 << 5;

/// Machine timer interrupt enable bit in `mie` register.
pub const MIE_MTIE: u64 = 1 << 7;

/// User external interrupt enable bit in `mie` register.
pub const MIE_UEIP: u64 = 1 << 8;

/// Supervisor external interrupt enable bit in `mie` register.
pub const MIE_SEIP: u64 = 1 << 9;

/// Machine external interrupt enable bit in `mie` register.
pub const MIE_MEIP: u64 = 1 << 11;

/// User software interrupt pending bit in `mip` register.
pub const MIP_USIP: u64 = 1 << 0;

/// Supervisor software interrupt pending bit in `mip` register.
pub const MIP_SSIP: u64 = 1 << 1;

/// Machine software interrupt pending bit in `mip` register.
pub const MIP_MSIP: u64 = 1 << 3;

/// User timer interrupt pending bit in `mip` register.
pub const MIP_UTIP: u64 = 1 << 4;

/// Supervisor timer interrupt pending bit in `mip` register.
pub const MIP_STIP: u64 = 1 << 5;

/// Machine timer interrupt pending bit in `mip` register.
pub const MIP_MTIP: u64 = 1 << 7;

/// User external interrupt pending bit in `mip` register.
pub const MIP_UEIP: u64 = 1 << 8;

/// Supervisor external interrupt pending bit in `mip` register.
pub const MIP_SEIP: u64 = 1 << 9;

/// Machine external interrupt pending bit in `mip` register.
pub const MIP_MEIP: u64 = 1 << 11;

/// Simulation panic CSR address (custom, for debugging).
pub const CSR_SIM_PANIC: CsrAddr = CsrAddr::from_u32(0x8FF);

/// Supervisor previous interrupt enable bit in `mstatus` register.
pub const MSTATUS_SPIE: u64 = 1 << 5;

/// Machine previous interrupt enable bit in `mstatus` register.
pub const MSTATUS_MPIE: u64 = 1 << 7;

/// Supervisor previous privilege mode bit in `mstatus` register.
pub const MSTATUS_SPP: u64 = 1 << 8;

/// Machine previous privilege mode field mask in `mstatus` register.
pub const MSTATUS_MPP: u64 = 3 << 11;

/// Bit shift for machine previous privilege mode field in `mstatus` register.
pub const MSTATUS_MPP_SHIFT: u64 = 11;

/// Bit mask for machine previous privilege mode field in `mstatus` register.
pub const MSTATUS_MPP_MASK: u64 = 3;

/// Floating-point state field mask in `mstatus` register.
pub const MSTATUS_FS: u64 = 3 << 13;

/// Floating-point state: off (no FPU state).
pub const MSTATUS_FS_OFF: u64 = 0 << 13;

/// Floating-point state: initial (FPU state is initial).
pub const MSTATUS_FS_INIT: u64 = 1 << 13;

/// Floating-point state: clean (FPU state is clean, no writes).
pub const MSTATUS_FS_CLEAN: u64 = 2 << 13;

/// Floating-point state: dirty (FPU state has been modified).
pub const MSTATUS_FS_DIRTY: u64 = 3 << 13;

/// Vector extension state field mask in `mstatus` register (bits 10:9).
pub const MSTATUS_VS: u64 = 3 << 9;

/// Vector state: off (vector unit disabled).
pub const MSTATUS_VS_OFF: u64 = 0 << 9;

/// Vector state: initial (vector state present but clean).
pub const MSTATUS_VS_INIT: u64 = 1 << 9;

/// Vector state: clean (vector state not modified since last save).
pub const MSTATUS_VS_CLEAN: u64 = 2 << 9;

/// Vector state: dirty (vector state has been modified).
pub const MSTATUS_VS_DIRTY: u64 = 3 << 9;

/// SD (State Dirty) summary bit in `mstatus`/`sstatus` (bit 63 for RV64).
/// Set when FS, VS, or XS is Dirty.
pub const MSTATUS_SD: u64 = 1 << 63;

/// `MPRV` (Modify `PRiVilege`) bit in `mstatus` register (bit 17).
/// When set, loads/stores use the privilege in MPP instead of current privilege.
pub const MSTATUS_MPRV: u64 = 1 << 17;

/// Supervisor user memory access bit in `mstatus` register.
pub const MSTATUS_SUM: u64 = 1 << 18;

/// Make executable readable bit in `mstatus` register.
pub const MSTATUS_MXR: u64 = 1 << 19;

/// Trap Virtual Memory bit in `mstatus` register (bit 20).
/// When set, attempts to read/write `satp` or execute SFENCE.VMA in S-mode will trap.
pub const MSTATUS_TVM: u64 = 1 << 20;

/// Timeout Wait bit in `mstatus` register (bit 21).
/// When set, WFI executed in S-mode will trap after an implementation-defined timeout.
pub const MSTATUS_TW: u64 = 1 << 21;

/// Trap SRET bit in `mstatus` register (bit 22).
/// When set, SRET executed in S-mode will raise an illegal instruction exception.
pub const MSTATUS_TSR: u64 = 1 << 22;

/// User XLEN field in `mstatus` register (bits 33:32).
pub const MSTATUS_UXL: u64 = 3 << 32;

/// Supervisor XLEN field in `mstatus` register (bits 35:34).
pub const MSTATUS_SXL: u64 = 3 << 34;

/// Bit shift for address translation mode field in `satp` register.
pub const SATP_MODE_SHIFT: u64 = 60;

/// Bare (no address translation) mode value for `satp` register.
pub const SATP_MODE_BARE: u64 = 0;

/// SV39 (39-bit virtual address) mode value for `satp` register.
pub const SATP_MODE_SV39: u64 = 8;

/// Bit mask for address translation mode field in `satp` register.
pub const SATP_MODE_MASK: u64 = 0xF;

/// Physical page number mask in `satp` register.
pub const SATP_PPN_MASK: u64 = 0xFFF_FFFF_FFFF;

/// Bit shift for ASID field in `satp` register (bits [59:44]).
pub const SATP_ASID_SHIFT: u64 = 44;

/// Bit mask for ASID field in `satp` register (16 bits).
pub const SATP_ASID_MASK: u64 = 0xFFFF;

/// MISA extension bit for atomic operations (A extension).
pub const MISA_EXT_A: u64 = 1 << 0;

/// MISA extension bit for compressed instructions (C extension).
pub const MISA_EXT_C: u64 = 1 << 2;

/// MISA extension bit for double-precision floating-point (D extension).
pub const MISA_EXT_D: u64 = 1 << 3;

/// MISA extension bit for single-precision floating-point (F extension).
pub const MISA_EXT_F: u64 = 1 << 5;

/// MISA extension bit for base integer instructions (I extension).
pub const MISA_EXT_I: u64 = 1 << 8;

/// MISA extension bit for integer multiply/divide (M extension).
pub const MISA_EXT_M: u64 = 1 << 12;

/// MISA extension bit for supervisor mode (S extension).
pub const MISA_EXT_S: u64 = 1 << 18;

/// MISA extension bit for user mode (U extension).
pub const MISA_EXT_U: u64 = 1 << 20;

/// MISA extension bit for vector operations (V extension).
pub const MISA_EXT_V: u64 = 1 << 21;

/// MISA XLEN field value for 32-bit architecture.
pub const MISA_XLEN_32: u64 = 1 << 62;

/// MISA XLEN field value for 64-bit architecture.
pub const MISA_XLEN_64: u64 = 2 << 62;

/// MISA XLEN field value for 128-bit architecture.
pub const MISA_XLEN_128: u64 = 3 << 62;

/// Default `mstatus` value for RV64 architecture.
pub const MSTATUS_DEFAULT_RV64: u64 = 0xa000_00000;

/// Default `misa` value for RV64IMAFDC architecture.
pub const MISA_DEFAULT_RV64IMAFDC: u64 = 0x8000_0000_0014_112D;

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
#[derive(Clone, Default, Debug)]
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
    /// Supervisor status (subset of `mstatus`).
    pub sstatus: u64,
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
    /// Cycle counter (user-readable).
    pub cycle: u64,
    /// Timer value (user-readable, derived from cycles).
    pub time: u64,
    /// Instructions retired counter (user-readable).
    pub instret: u64,
    /// Machine cycle counter.
    pub mcycle: u64,
    /// Machine instructions retired counter.
    pub minstret: u64,
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
}

impl Csrs {
    /// Reads a CSR value by its address.
    ///
    /// # Arguments
    ///
    /// * `addr` - The CSR address.
    ///
    /// # Returns
    ///
    /// The 64-bit value stored in the specified CSR, or 0 if the address is not recognized.
    pub const fn read(&self, addr: CsrAddr) -> u64 {
        match addr.as_u32() {
            x if x == FFLAGS.as_u32() => self.fflags & 0x1F,
            x if x == FRM.as_u32() => self.frm & 0x7,
            x if x == FCSR.as_u32() => ((self.frm & 0x7) << 5) | (self.fflags & 0x1F),
            x if x == MSTATUS.as_u32() => {
                let val = self.mstatus & !MSTATUS_SD;
                let fs_dirty = val & MSTATUS_FS == MSTATUS_FS_DIRTY;
                let vs_dirty = val & MSTATUS_VS == MSTATUS_VS_DIRTY;
                if fs_dirty || vs_dirty { val | MSTATUS_SD } else { val }
            }
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
            x if x == SSTATUS.as_u32() => {
                let val = self.sstatus & !MSTATUS_SD;
                let fs_dirty = val & MSTATUS_FS == MSTATUS_FS_DIRTY;
                let vs_dirty = val & MSTATUS_VS == MSTATUS_VS_DIRTY;
                if fs_dirty || vs_dirty { val | MSTATUS_SD } else { val }
            }
            x if x == SIE.as_u32() => self.sie,
            x if x == STVEC.as_u32() => self.stvec,
            x if x == SSCRATCH.as_u32() => self.sscratch,
            x if x == SEPC.as_u32() => self.sepc,
            x if x == SCAUSE.as_u32() => self.scause,
            x if x == STVAL.as_u32() => self.stval,
            x if x == SIP.as_u32() => self.sip,
            x if x == SATP.as_u32() => self.satp,
            x if x == CYCLE.as_u32() => self.cycle,
            x if x == TIME.as_u32() => self.time,
            x if x == INSTRET.as_u32() => self.instret,
            x if x == MCYCLE.as_u32() => self.mcycle,
            x if x == MINSTRET.as_u32() => self.minstret,
            x if x == MCOUNTEREN.as_u32() => self.mcounteren,
            x if x == SCOUNTEREN.as_u32() => self.scounteren,
            x if x == MENVCFG.as_u32() => self.menvcfg,
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
    ///
    /// # Arguments
    ///
    /// * `addr` - The CSR address.
    /// * `val` - The 64-bit value to write.
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
            x if x == SSTATUS.as_u32() => self.sstatus = val,
            x if x == SIE.as_u32() => self.sie = val,
            x if x == STVEC.as_u32() => self.stvec = val,
            x if x == SSCRATCH.as_u32() => self.sscratch = val,
            x if x == SEPC.as_u32() => self.sepc = val,
            x if x == SCAUSE.as_u32() => self.scause = val,
            x if x == STVAL.as_u32() => self.stval = val,
            x if x == SIP.as_u32() => self.sip = val,
            x if x == SATP.as_u32() => {
                let mode = (val >> SATP_MODE_SHIFT) & SATP_MODE_MASK;
                let new_mode = if mode == SATP_MODE_SV39 { SATP_MODE_SV39 } else { SATP_MODE_BARE };
                let mask = !(SATP_MODE_MASK << SATP_MODE_SHIFT);
                self.satp = (val & mask) | (new_mode << SATP_MODE_SHIFT);
            }
            x if x == CYCLE.as_u32() => self.cycle = val,
            x if x == TIME.as_u32() => self.time = val,
            x if x == INSTRET.as_u32() => self.instret = val,
            x if x == MCYCLE.as_u32() => self.mcycle = val,
            x if x == MINSTRET.as_u32() => self.minstret = val,
            x if x == MCOUNTEREN.as_u32() => self.mcounteren = val,
            x if x == SCOUNTEREN.as_u32() => self.scounteren = val,
            x if x == VSTART.as_u32() => self.vstart = val,
            x if x == VXSAT.as_u32() => self.vxsat = val & 0x1,
            x if x == VXRM.as_u32() => self.vxrm = val & 0x3,
            x if x == VCSR.as_u32() => {
                self.vxsat = val & 0x1;
                self.vxrm = (val >> 1) & 0x3;
            }
            // VL, VTYPE, VLENB are not writable via CSR instructions
            // (only writable by vsetvl family, handled in execute stage)
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        // SATP write with invalid mode falls back to BARE
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

        csrs.write(CsrAddr::from_u32(9999), 0x1); // invalid csr
        assert_eq!(csrs.read(CsrAddr::from_u32(9999)), 0x0); // returns 0
    }
}
