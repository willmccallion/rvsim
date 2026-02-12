//! Control and Status Register (CSR) definitions and operations.
//!
//! This module implements the CSR subsystem for the RISC-V processor. It provides:
//! 1. **Address Definitions:** Constants for all standard machine and supervisor CSRs.
//! 2. **Field Masks:** Bitmasks and shifts for status, ISA, and translation control.
//! 3. **Register Storage:** The `Csrs` struct for maintaining architectural state.
//! 4. **Access Logic:** Standardized read and write operations for register interaction.

/// Machine vendor ID CSR address.
pub const MVENDORID: u32 = 0xF11;

/// Machine architecture ID CSR address.
pub const MARCHID: u32 = 0xF12;

/// Machine implementation ID CSR address.
pub const MIMPID: u32 = 0xF13;

/// Machine hardware thread ID CSR address.
pub const MHARTID: u32 = 0xF14;

/// Machine status register CSR address.
pub const MSTATUS: u32 = 0x300;

/// Machine ISA register CSR address.
pub const MISA: u32 = 0x301;

/// Machine exception delegation register CSR address.
pub const MEDELEG: u32 = 0x302;

/// Machine interrupt delegation register CSR address.
pub const MIDELEG: u32 = 0x303;

/// Machine interrupt enable register CSR address.
pub const MIE: u32 = 0x304;

/// Machine trap vector base address register CSR address.
pub const MTVEC: u32 = 0x305;

/// Machine counter enable register CSR address.
pub const MCOUNTEREN: u32 = 0x306;

/// Machine scratch register CSR address.
pub const MSCRATCH: u32 = 0x340;

/// Machine exception program counter CSR address.
pub const MEPC: u32 = 0x341;

/// Machine cause register CSR address.
pub const MCAUSE: u32 = 0x342;

/// Machine trap value register CSR address.
pub const MTVAL: u32 = 0x343;

/// Machine interrupt pending register CSR address.
pub const MIP: u32 = 0x344;

/// Supervisor status register CSR address.
pub const SSTATUS: u32 = 0x100;

/// Supervisor interrupt enable register CSR address.
pub const SIE: u32 = 0x104;

/// Supervisor trap vector base address register CSR address.
pub const STVEC: u32 = 0x105;

/// Supervisor counter enable register CSR address.
pub const SCOUNTEREN: u32 = 0x106;

/// Supervisor scratch register CSR address.
pub const SSCRATCH: u32 = 0x140;

/// Supervisor exception program counter CSR address.
pub const SEPC: u32 = 0x141;

/// Supervisor cause register CSR address.
pub const SCAUSE: u32 = 0x142;

/// Supervisor trap value register CSR address.
pub const STVAL: u32 = 0x143;

/// Supervisor interrupt pending register CSR address.
pub const SIP: u32 = 0x144;

/// Supervisor address translation and protection register CSR address.
pub const SATP: u32 = 0x180;

/// Supervisor timer compare register CSR address.
pub const STIMECMP: u32 = 0x14D;

/// Cycle counter CSR address (read-only, user mode accessible).
pub const CYCLE: u32 = 0xC00;

/// Real-time counter CSR address (read-only, user mode accessible).
pub const TIME: u32 = 0xC01;

/// Instructions retired counter CSR address (read-only, user mode accessible).
pub const INSTRET: u32 = 0xC02;

/// Machine cycle counter CSR address.
pub const MCYCLE: u32 = 0xB00;

/// Machine instructions retired counter CSR address.
pub const MINSTRET: u32 = 0xB02;

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
pub const CSR_SIM_PANIC: u32 = 0x8FF;

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

/// Supervisor user memory access bit in `mstatus` register.
pub const MSTATUS_SUM: u64 = 1 << 18;

/// Make executable readable bit in `mstatus` register.
pub const MSTATUS_MXR: u64 = 1 << 19;

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

/// MISA XLEN field value for 32-bit architecture.
pub const MISA_XLEN_32: u64 = 1 << 62;

/// MISA XLEN field value for 64-bit architecture.
pub const MISA_XLEN_64: u64 = 2 << 62;

/// MISA XLEN field value for 128-bit architecture.
pub const MISA_XLEN_128: u64 = 3 << 62;

/// Default `mstatus` value for RV64 architecture.
pub const MSTATUS_DEFAULT_RV64: u64 = 0xa000_00000;

/// Default `misa` value for RV64IMAFDC architecture.
pub const MISA_DEFAULT_RV64IMAFDC: u64 = 0x8000_0000_0014_1101;

/// Control and Status Register file.
///
/// Contains all machine-level and supervisor-level CSRs that control processor state,
/// interrupt handling, memory management, and performance counters.
#[derive(Clone, Default)]
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
}

impl Csrs {
    /// Reads a CSR value by its address.
    ///
    /// # Arguments
    ///
    /// * `addr` - The 12-bit CSR address.
    ///
    /// # Returns
    ///
    /// The 64-bit value stored in the specified CSR, or 0 if the address is not recognized.
    pub fn read(&self, addr: u32) -> u64 {
        match addr {
            MSTATUS => self.mstatus,
            MISA => self.misa,
            MEDELEG => self.medeleg,
            MIDELEG => self.mideleg,
            MIE => self.mie,
            MTVEC => self.mtvec,
            MSCRATCH => self.mscratch,
            MEPC => self.mepc,
            MCAUSE => self.mcause,
            MTVAL => self.mtval,
            MIP => self.mip,
            SSTATUS => self.sstatus,
            SIE => self.sie,
            STVEC => self.stvec,
            SSCRATCH => self.sscratch,
            SEPC => self.sepc,
            SCAUSE => self.scause,
            STVAL => self.stval,
            SIP => self.sip,
            SATP => self.satp,
            CYCLE => self.cycle,
            TIME => self.time,
            INSTRET => self.instret,
            MCYCLE => self.mcycle,
            MINSTRET => self.minstret,
            _ => 0,
        }
    }

    /// Writes a value to a CSR by its address.
    ///
    /// # Arguments
    ///
    /// * `addr` - The 12-bit CSR address.
    /// * `val` - The 64-bit value to write.
    pub fn write(&mut self, addr: u32, val: u64) {
        match addr {
            MSTATUS => self.mstatus = val,
            MISA => self.misa = val,
            MEDELEG => self.medeleg = val,
            MIDELEG => self.mideleg = val,
            MIE => self.mie = val,
            MTVEC => self.mtvec = val,
            MSCRATCH => self.mscratch = val,
            MEPC => self.mepc = val,
            MCAUSE => self.mcause = val,
            MTVAL => self.mtval = val,
            MIP => self.mip = val,
            SSTATUS => self.sstatus = val,
            SIE => self.sie = val,
            STVEC => self.stvec = val,
            SSCRATCH => self.sscratch = val,
            SEPC => self.sepc = val,
            SCAUSE => self.scause = val,
            STVAL => self.stval = val,
            SIP => self.sip = val,
            SATP => {
                let mode = (val >> SATP_MODE_SHIFT) & SATP_MODE_MASK;
                let new_mode = if mode == SATP_MODE_SV39 {
                    SATP_MODE_SV39
                } else {
                    SATP_MODE_BARE
                };
                let mask = !(SATP_MODE_MASK << SATP_MODE_SHIFT);
                self.satp = (val & mask) | (new_mode << SATP_MODE_SHIFT);
            }
            CYCLE => self.cycle = val,
            TIME => self.time = val,
            INSTRET => self.instret = val,
            MCYCLE => self.mcycle = val,
            MINSTRET => self.minstret = val,
            _ => {}
        }
    }
}
