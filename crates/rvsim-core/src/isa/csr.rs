//! CSRs as the privileged spec defines them: addresses (the 12-bit field
//! in bits 31:20 of CSR instructions) and the layouts of their fields.

/// A 12-bit CSR (Control and Status Register) address (0x000–0xFFF).
///
/// CSR addresses are encoded as a 12-bit immediate in the instruction
/// word (bits 31:20). Bits above 12 are always zero.
///
/// # Example
///
/// ```ignore
/// use rvsim_core::arch::csr;
///
/// let val = cpu.csr_read(csr::SATP);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct CsrAddr(u16);

impl CsrAddr {
    /// Creates a `CsrAddr` from a raw `u16`, masking to 12 bits.
    ///
    /// Values above `0xFFF` are truncated.
    #[inline(always)]
    pub const fn new(addr: u16) -> Self {
        Self(addr & 0xFFF)
    }

    /// Creates a `CsrAddr` from a `u32` CSR constant.
    ///
    /// This is the primary conversion when working with the existing
    /// `u32` CSR address constants and the output of `InstructionBits::csr()`.
    ///
    /// Values above `0xFFF` are truncated (the high bits are always
    /// zero for valid CSR addresses).
    #[inline(always)]
    pub const fn from_u32(addr: u32) -> Self {
        Self((addr & 0xFFF) as u16)
    }

    /// Returns the address as a `u32` for use in match arms and PMP range checks.
    #[inline(always)]
    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }

    /// Returns the raw `u16` value.
    #[inline(always)]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Extracts the privilege level encoded in bits 9:8 of the CSR address.
    ///
    /// Per the RISC-V privileged spec (§2.1):
    /// - `0b00` = Unprivileged / User
    /// - `0b01` = Supervisor
    /// - `0b10` = Hypervisor
    /// - `0b11` = Machine
    #[inline(always)]
    pub const fn privilege_level(self) -> u8 {
        ((self.0 >> 8) & 0x3) as u8
    }

    /// Returns `true` if the CSR is read-only.
    ///
    /// Per the RISC-V privileged spec (§2.1): a CSR is read-only when
    /// bits 11:10 of its address are both `1` (`0b11`).
    #[inline(always)]
    pub const fn is_read_only(self) -> bool {
        ((self.0 >> 10) & 0x3) == 0x3
    }
}

impl From<u16> for CsrAddr {
    #[inline(always)]
    fn from(v: u16) -> Self {
        Self::new(v)
    }
}

impl From<CsrAddr> for u32 {
    #[inline(always)]
    fn from(c: CsrAddr) -> Self {
        c.0 as Self
    }
}

impl std::fmt::Display for CsrAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CSR({:#05x})", self.0)
    }
}

impl std::fmt::LowerHex for CsrAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::LowerHex::fmt(&self.0, f)
    }
}

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

/// ADUE bit in menvcfg — enables Svadu's hardware PTE A/D updates.
pub const MENVCFG_ADUE: u64 = 1 << 61;

/// CBZE bit in menvcfg — enables Zicboz cbo.zero in S/U modes.
pub const MENVCFG_CBZE: u64 = 1 << 7;

/// Supervisor environment configuration register CSR address.
pub const SENVCFG: CsrAddr = CsrAddr::from_u32(0x10A);

/// CBZE bit in senvcfg — enables Zicboz cbo.zero in U mode.
pub const SENVCFG_CBZE: u64 = 1 << 7;

/// CBCFE bit in menvcfg — enables Zicbom cbo.clean / cbo.flush in S/U.
pub const MENVCFG_CBCFE: u64 = 1 << 6;

/// CBCFE bit in senvcfg — enables Zicbom cbo.clean / cbo.flush in U.
pub const SENVCFG_CBCFE: u64 = 1 << 6;

/// CBIE field shift in menvcfg / senvcfg (bits 5:4).
pub const MENVCFG_CBIE_SHIFT: u32 = 4;

/// CBIE field mask (2 bits).
pub const MENVCFG_CBIE_MASK: u64 = 0b11;

/// FIOM bit of menvcfg / senvcfg: FENCE on device I/O also orders memory.
pub const ENVCFG_FIOM: u64 = 1 << 0;

/// The senvcfg fields this hart implements: FIOM and the Zicbom/Zicboz
/// enables.
pub const SENVCFG_WRITABLE: u64 = ENVCFG_FIOM | ENVCFG_CBIE | SENVCFG_CBCFE | SENVCFG_CBZE;

/// The menvcfg fields this hart implements: those of senvcfg plus Sstc's STCE.
pub const MENVCFG_WRITABLE: u64 = SENVCFG_WRITABLE | MENVCFG_STCE;

/// CBIE encoding: cbo.inval is illegal in S/U.
pub const CBIE_ILLEGAL: u64 = 0b00;

/// CBIE encoding: cbo.inval is permitted but executes as cbo.flush.
pub const CBIE_FLUSH: u64 = 0b01;

/// CBIE encoding: cbo.inval performs full invalidate (may discard dirty data).
pub const CBIE_INVAL: u64 = 0b11;

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

/// `mcountinhibit.CY`: stops `mcycle`.
pub const MCOUNTINHIBIT_CY: u64 = 1;

/// `mcountinhibit.IR`: stops `minstret`.
pub const MCOUNTINHIBIT_IR: u64 = 1 << 2;

/// The `mcountinhibit` bits that exist: CY and IR (TM is hardwired zero and
/// no hardware performance counters count).
pub const MCOUNTINHIBIT_WRITABLE: u64 = MCOUNTINHIBIT_CY | MCOUNTINHIBIT_IR;

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

/// Trigger select register (Sdtrig).
pub const TSELECT: CsrAddr = CsrAddr::from_u32(0x7A0);

/// Trigger data 1 register (Sdtrig).
pub const TDATA1: CsrAddr = CsrAddr::from_u32(0x7A1);

/// Trigger data 2 register (Sdtrig).
pub const TDATA2: CsrAddr = CsrAddr::from_u32(0x7A2);

/// Trigger data 3 register (Sdtrig).
pub const TDATA3: CsrAddr = CsrAddr::from_u32(0x7A3);

/// Trigger info register (Sdtrig).
pub const TINFO: CsrAddr = CsrAddr::from_u32(0x7A4);

/// Trigger control register (Sdtrig).
pub const TCONTROL: CsrAddr = CsrAddr::from_u32(0x7A5);

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

/// Fields of `mstatus` that `sstatus` exposes and lets software write.
pub const SSTATUS_WRITABLE: u64 =
    MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP | MSTATUS_VS | MSTATUS_FS | MSTATUS_SUM | MSTATUS_MXR;

/// Fields of `mstatus` visible through `sstatus`.
pub const SSTATUS_VISIBLE: u64 = SSTATUS_WRITABLE | MSTATUS_UXL;

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

/// SV48 (48-bit virtual address) mode value for `satp` register.
pub const SATP_MODE_SV48: u64 = 9;

/// SV57 (57-bit virtual address) mode value for `satp` register.
pub const SATP_MODE_SV57: u64 = 10;

/// Bit mask for address translation mode field in `satp` register.
pub const SATP_MODE_MASK: u64 = 0xF;

/// Physical page number mask in `satp` register.
pub const SATP_PPN_MASK: u64 = 0xFFF_FFFF_FFFF;

/// Bit shift for ASID field in `satp` register (bits \[59:44\]).
pub const SATP_ASID_SHIFT: u64 = 44;

/// Bit mask for ASID field in `satp` register (16 bits).
pub const SATP_ASID_MASK: u64 = 0xFFFF;

/// MISA extension bit for atomic operations (A extension).
pub const MISA_EXT_A: u64 = 1 << 0;

/// MISA extension bit for bit manipulation (B: Zba, Zbb and Zbs).
pub const MISA_EXT_B: u64 = 1 << 1;

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

/// Default `misa` value: RV64IMAFDC with B, S and U.
pub const MISA_DEFAULT_RV64GCB: u64 = 0x8000_0000_0014_112F;

/// The CBIE field of menvcfg / senvcfg in place.
pub const ENVCFG_CBIE: u64 = MENVCFG_CBIE_MASK << MENVCFG_CBIE_SHIFT;

/// CBIE's reserved encoding.
pub const CBIE_RESERVED: u64 = 0b10;
