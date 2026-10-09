//! The commit log a lockstep checker replays against a reference model:
//! the architectural state the log starts from, then one line per retired
//! instruction with its architectural effects and one per trap taken.
//!
//! The log opens with `core   0: reset pc 0x<pc> priv <mode>` and a `core
//! 0: reset x<n>|f<n>|c<csr> 0x<value>` line for each nonzero register and
//! for each CSR in [`RESET_CSRS`].
//!
//! An instruction's line is `core   0: 0x<pc> (0x<inst>) priv <mode>`
//! followed by whichever of these it has, in this order: `x<n> 0x<value>`
//! or `f<n> 0x<value>` (its destination register), `c<csr> 0x<value>` (a
//! CSR it wrote, read back after the write), `c001 0x<fflags>` (when it
//! raised FP flags), `load 0x<vaddr> 0x<paddr> <bytes> 0x<raw>`, `store
//! 0x<vaddr> 0x<paddr> <bytes> 0x<data>` and `vec` (a vector instruction,
//! whose vector register and memory effects are not logged). A faulting
//! instruction has no effects. A trap is `core   0: trap 0x<cause> 0x<epc>
//! 0x<tval>`, with the interrupt bit in `cause`.

use std::io::{self, Write};

use crate::common::{PhysAddr, VirtAddr};
use crate::exec::compute::amo::atomic_alu;
use crate::isa::csr::{self, CsrAddr};
use crate::isa::op::{AtomicOp, MemWidth, VectorOp};
use crate::isa::privileged::PrivilegeMode;
use crate::isa::reg::RegIdx;
use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::latches::Mem1Mem2Entry;
use crate::uarch::pipeline::rob::RobEntry;

/// The CSRs the log's reset lines give, so a reference model can start from
/// the same machine state.
pub const RESET_CSRS: [CsrAddr; 16] = [
    csr::MSTATUS,
    csr::MISA,
    csr::MEDELEG,
    csr::MIDELEG,
    csr::MIE,
    csr::MTVEC,
    csr::MCOUNTEREN,
    csr::MENVCFG,
    csr::MSCRATCH,
    csr::STVEC,
    csr::SCOUNTEREN,
    csr::SENVCFG,
    csr::SSCRATCH,
    csr::SATP,
    csr::FCSR,
    csr::VTYPE,
];

/// Writes the reset lines: hart 0's PC, privilege mode, nonzero registers
/// and [`RESET_CSRS`].
///
/// # Errors
///
/// Returns the error writing to `out` raised.
pub fn write_reset(out: &mut impl Write, ctx: &CoreCtx<'_>) -> io::Result<()> {
    writeln!(out, "core   0: reset pc 0x{:016x} priv {}", ctx.hart.pc, ctx.hart.privilege.to_u8())?;
    for index in 1..32u8 {
        let value = ctx.hart.regs.read(RegIdx::new(index));
        if value != 0 {
            writeln!(out, "core   0: reset x{index} 0x{value:016x}")?;
        }
    }
    for index in 0..32u8 {
        let value = ctx.hart.regs.read_f(RegIdx::new(index));
        if value != 0 {
            writeln!(out, "core   0: reset f{index} 0x{value:016x}")?;
        }
    }
    for addr in RESET_CSRS {
        writeln!(out, "core   0: reset c{:03x} 0x{:016x}", addr.as_u32(), ctx.csr_read(addr))?;
    }
    Ok(())
}

/// A scalar memory access an instruction performed: what it read, what it
/// wrote, or both for an AMO.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemEffect {
    /// The address the instruction computed.
    pub vaddr: VirtAddr,
    /// Where it translated to.
    pub paddr: PhysAddr,
    /// The access width.
    pub width: MemWidth,
    /// The raw bytes read, before sign extension or NaN-boxing.
    pub read: Option<u64>,
    /// The bytes written.
    pub write: Option<u64>,
}

impl MemEffect {
    /// The access a load, LR, SC or AMO leaving memory2 performed. A plain
    /// store's data can arrive after memory2, so commit takes it from the
    /// store buffer instead.
    #[must_use]
    pub fn of_memory2(mem: &Mem1Mem2Entry) -> Option<Self> {
        if mem.trap.is_some() || mem.vec_mem.is_some() || mem.ctrl.system_op.is_cbo() {
            return None;
        }
        let width = mem.ctrl.width;
        let (read, write) = match mem.ctrl.atomic_op {
            Some(AtomicOp::Lr) => (Some(mem.load_data), None),
            Some(AtomicOp::Sc) => (None, (mem.load_data == 0).then_some(mem.store_data)),
            Some(op) => {
                (Some(mem.load_data), Some(atomic_alu(op, mem.load_data, mem.store_data, width)))
            }
            None if mem.ctrl.mem_read => (Some(mem.load_data), None),
            None => return None,
        };
        Some(Self::new(mem.vaddr, mem.paddr, width, read, write))
    }

    /// An access of `width` at `vaddr`/`paddr`, its values cut to `width`.
    #[must_use]
    pub const fn new(
        vaddr: VirtAddr,
        paddr: PhysAddr,
        width: MemWidth,
        read: Option<u64>,
        write: Option<u64>,
    ) -> Self {
        let mask = width_mask(width);
        Self {
            vaddr,
            paddr,
            width,
            read: match read {
                Some(value) => Some(value & mask),
                None => None,
            },
            write: match write {
                Some(value) => Some(value & mask),
                None => None,
            },
        }
    }
}

const fn width_mask(width: MemWidth) -> u64 {
    match width.bytes() {
        8 => u64::MAX,
        bytes => (1u64 << (bytes * 8)) - 1,
    }
}

/// What a retired instruction did, taken before its retirement changes the
/// privilege mode.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct Retired {
    pc: u64,
    inst: u32,
    privilege: PrivilegeMode,
    destination: Option<Destination>,
    csr: Option<CsrAddr>,
    raised_fp_flags: bool,
    mem: Option<MemEffect>,
    vector: bool,
}

#[derive(Clone, Copy, Debug)]
struct Destination {
    file: RegisterFile,
    index: usize,
    value: u64,
}

#[derive(Clone, Copy, Debug)]
enum RegisterFile {
    Integer,
    Float,
}

impl RegisterFile {
    const fn prefix(self) -> char {
        match self {
            Self::Integer => 'x',
            Self::Float => 'f',
        }
    }
}

impl Retired {
    /// Takes `entry`'s effects as it retires in `privilege`; `store` is the
    /// write a plain store left in the store buffer.
    pub fn capture(entry: &RobEntry, privilege: PrivilegeMode, store: Option<MemEffect>) -> Self {
        let value = entry.result.unwrap_or(0);
        let destination = if entry.ctrl.fp_reg_write {
            Some(Destination { file: RegisterFile::Float, index: entry.rd.as_usize(), value })
        } else if entry.ctrl.reg_write && !entry.rd.is_zero() {
            Some(Destination { file: RegisterFile::Integer, index: entry.rd.as_usize(), value })
        } else {
            None
        };
        Self {
            pc: entry.pc,
            inst: entry.inst,
            privilege,
            destination,
            csr: entry.csr_update.as_ref().map(|update| update.addr),
            raised_fp_flags: entry.fp_flags != 0,
            mem: entry.mem_effect.or(store),
            vector: entry.ctrl.vec_op != VectorOp::None,
        }
    }

    /// The CSR the instruction wrote, whose value the line reports.
    #[must_use]
    pub const fn csr(&self) -> Option<CsrAddr> {
        self.csr
    }

    /// Whether the instruction raised FP flags, so the line reports
    /// `fflags`.
    #[must_use]
    pub const fn raised_fp_flags(&self) -> bool {
        self.raised_fp_flags
    }

    /// Writes the instruction's line; `csr_value` is the written CSR's
    /// value after the write and `fflags` the flags after accruing its own.
    ///
    /// # Errors
    ///
    /// Returns the error writing to `out` raised.
    pub fn write(
        &self,
        out: &mut impl Write,
        csr_value: Option<u64>,
        fflags: Option<u64>,
    ) -> io::Result<()> {
        write_header(out, self.pc, self.inst, self.privilege)?;
        if let Some(dest) = self.destination {
            write!(out, " {}{} 0x{:016x}", dest.file.prefix(), dest.index, dest.value)?;
        }
        if let (Some(addr), Some(value)) = (self.csr, csr_value) {
            write!(out, " c{:03x} 0x{value:016x}", addr.as_u32())?;
        }
        if let Some(flags) = fflags {
            write!(out, " c{:03x} 0x{flags:016x}", csr::FFLAGS.as_u32())?;
        }
        if let Some(mem) = self.mem {
            let bytes = mem.width.bytes();
            if let Some(read) = mem.read {
                write!(
                    out,
                    " load 0x{:016x} 0x{:016x} {bytes} 0x{read:016x}",
                    mem.vaddr.val(),
                    mem.paddr.val()
                )?;
            }
            if let Some(written) = mem.write {
                write!(
                    out,
                    " store 0x{:016x} 0x{:016x} {bytes} 0x{written:016x}",
                    mem.vaddr.val(),
                    mem.paddr.val()
                )?;
            }
        }
        if self.vector {
            write!(out, " vec")?;
        }
        writeln!(out)
    }
}

/// Writes the line of an instruction that faulted at `pc`.
///
/// # Errors
///
/// Returns the error writing to `out` raised.
pub fn write_faulted(
    out: &mut impl Write,
    pc: u64,
    inst: u32,
    privilege: PrivilegeMode,
) -> io::Result<()> {
    write_header(out, pc, inst, privilege)?;
    writeln!(out)
}

/// Writes the line of a trap taken with `cause` (interrupt bit included)
/// at `epc`.
///
/// # Errors
///
/// Returns the error writing to `out` raised.
pub fn write_trap(out: &mut impl Write, cause: u64, epc: u64, tval: u64) -> io::Result<()> {
    writeln!(out, "core   0: trap 0x{cause:016x} 0x{epc:016x} 0x{tval:016x}")
}

fn write_header(
    out: &mut impl Write,
    pc: u64,
    inst: u32,
    privilege: PrivilegeMode,
) -> io::Result<()> {
    write!(out, "core   0: 0x{pc:016x} (0x{inst:08x}) priv {}", privilege.to_u8())
}
