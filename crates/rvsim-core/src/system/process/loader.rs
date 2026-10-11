//! Loads a static Linux ELF as a user process, and starts threads.
//!
//! Its segments go into pages behind page tables the loader builds, the
//! initial stack is what Linux's ELF loader builds (argc, argv, envp and
//! the auxiliary vector), and its first thread starts at the entry in
//! user mode. A cloned thread enters user mode the same way, with a copy
//! of its parent's registers.

use object::Endianness;
use object::elf::{EM_RISCV, ET_DYN, ET_EXEC, PF_R, PF_W, PF_X, PT_INTERP, PT_LOAD, PT_PHDR};
use object::read::elf::{ElfFile64, FileHeader, ProgramHeader};

use super::{
    AddressSpace, FrameAllocator, PAGE_SIZE, Permissions, Process, Processes, SpaceError, Wakeup,
    WakeupKind, syscall,
};
use crate::arch::Hart;
use crate::common::{CoreId, HartId, PhysAddr, SimError, VirtAddr};
use crate::isa::csr;
use crate::isa::privileged::{PagingMode, PrivilegeMode};
use crate::isa::reg::{self, RegIdx};
use crate::sim::memory::GlobalMemory;
use crate::soc::terminal::{InputReader, SharedTerminal};
use crate::system::state::SystemState;
use crate::uarch::ctx::CoreCtx;

const AT_NULL: u64 = 0;
const AT_PHDR: u64 = 3;
const AT_PHENT: u64 = 4;
const AT_PHNUM: u64 = 5;
const AT_PAGESZ: u64 = 6;
const AT_BASE: u64 = 7;
const AT_FLAGS: u64 = 8;
const AT_ENTRY: u64 = 9;
const AT_UID: u64 = 11;
const AT_EUID: u64 = 12;
const AT_GID: u64 = 13;
const AT_EGID: u64 = 14;
const AT_PLATFORM: u64 = 15;
const AT_HWCAP: u64 = 16;
const AT_CLKTCK: u64 = 17;
const AT_SECURE: u64 = 23;
const AT_RANDOM: u64 = 25;
const AT_EXECFN: u64 = 31;

/// `AT_HWCAP` as Linux reports a `rv64imafdc` core: one bit per ISA letter.
const HWCAP_IMAFDC: u64 = (1 << 8) | (1 << 12) | (1 << 0) | (1 << 5) | (1 << 3) | (1 << 2);
const CLOCK_TICKS_PER_SECOND: u64 = 100;
/// `pmpaddr` all ones: with NAPOT, every address.
const PMP_NAPOT_EVERYTHING: u64 = (1 << 54) - 1;
/// `pmpcfg` entry 0: readable, writable, executable, NAPOT.
const PMP_RWX_NAPOT: u64 = 0x1F;
/// Where a static position-independent executable is loaded: the address
/// static executables are linked at.
pub const PIE_BASE: VirtAddr = VirtAddr::new(0x1_0000);
/// Every counter readable from user mode, as under gem5 and qemu.
const COUNTERS_ALL: u64 = 0xFFFF_FFFF;

/// The program's command line and environment.
#[derive(Debug, Clone, Default)]
pub struct ProcessArgs {
    /// `argv`, `argv[0]` included.
    pub argv: Vec<String>,
    /// `envp` entries as `NAME=value`.
    pub envp: Vec<String>,
}

/// A program to load: its ELF image and command line.
#[derive(Debug, Clone, Copy)]
pub struct ProgramImage<'a> {
    /// The static riscv64 Linux ELF.
    pub data: &'a [u8],
    /// Its command line and environment.
    pub args: &'a ProcessArgs,
}

/// Loads `programs`, static riscv64 Linux ELFs, as processes whose first
/// threads run one per hart from hart 0.
///
/// Each runs under `mode` paging in its own address space drawn from
/// RAM's frames; a hart beyond the programs is parked, and the run ends
/// when the last program exits.
///
/// # Errors
///
/// [`SimError::NotAnElf`], [`SimError::UnsupportedProgram`] or
/// [`SimError::ProcessMemory`].
pub fn load_processes(
    state: &mut SystemState,
    programs: &[ProgramImage<'_>],
    mode: PagingMode,
    clock_hz: u64,
) -> Result<(), SimError> {
    let harts = state.harts.len();
    if programs.is_empty() || programs.len() > harts {
        return Err(SimError::UnsupportedProgram {
            reason: format!(
                "{} programs need {} harts to run on, one each; configure hart_count",
                programs.len(),
                programs.len()
            ),
        });
    }
    if mode == PagingMode::Bare {
        return Err(SimError::UnsupportedProgram {
            reason: "SE mode needs a paging mode (Sv39, Sv48 or Sv57)".to_string(),
        });
    }
    let images =
        programs.iter().map(|program| Image::parse(program.data)).collect::<Result<Vec<_>, _>>()?;

    let ram_base = state.config.system.ram_base;
    let ram_end = ram_base + state.config.memory.ram_size as u64;
    let mut frames = FrameAllocator::new(PhysAddr::new(ram_base), PhysAddr::new(ram_end));
    let mut loaded = Vec::with_capacity(programs.len());
    for (program, image) in programs.iter().zip(&images) {
        let terminal = state.uncore.terminal.clone();
        let memory = &mut state.uncore.memory;
        let (process, sp) =
            place(image, program.args, mode, clock_hz, terminal, memory, &mut frames)?;
        loaded.push((process, sp, image.entry));
    }

    let mut processes = Processes::new(harts, frames);
    for (index, (process, sp, entry)) in loaded.into_iter().enumerate() {
        let hart_id = state.harts[index].hart_id;
        let satp = process.space.satp();
        let _ = processes.add_program(process, hart_id);
        start_hart(state, hart_id, entry, sp, satp);
    }
    for hart in state.harts.iter_mut().skip(programs.len()) {
        park(hart);
    }
    state.uncore.processes = processes;
    state.uncore.read_console_through(InputReader::Proxy);
    Ok(())
}

/// Builds `image`'s address space and stack from `frames` and returns the
/// process and its initial stack pointer.
fn place(
    image: &Image<'_>,
    args: &ProcessArgs,
    mode: PagingMode,
    clock_hz: u64,
    terminal: SharedTerminal,
    memory: &mut GlobalMemory,
    frames: &mut FrameAllocator,
) -> Result<(Process, VirtAddr), SimError> {
    let mut space = AddressSpace::new(mode, memory, frames).map_err(space_error)?;
    let mut image_end = VirtAddr::new(0);
    for segment in &image.segments {
        let perms = segment.permissions;
        let mut page = segment.vaddr.page_floor();
        let end = segment.vaddr.checked_add(segment.memsz).ok_or_else(|| {
            SimError::UnsupportedProgram { reason: "a segment wraps the address space".to_string() }
        })?;
        while page < end {
            let merged = space.permissions(page).map_or(perms, |existing| existing.union(perms));
            space.map(memory, frames, page, merged).map_err(space_error)?;
            page = page.wrapping_add(PAGE_SIZE);
        }
        space.write(memory, segment.vaddr, segment.data).map_err(space_error)?;
        image_end = image_end.max(end);
    }

    let mut process = Process::new(space, image_end.page_ceil(), clock_hz, terminal);
    let stack_bytes = process.stack_bytes();
    let stack_bottom = process.stack_top().wrapping_sub(stack_bytes);
    process
        .space
        .map_range(memory, frames, stack_bottom, stack_bytes, Permissions::RW)
        .map_err(space_error)?;
    let sp = build_stack(&mut process, memory, image, args).map_err(space_error)?;
    Ok((process, sp))
}

/// Points `hart_id` at `entry` in user mode with `sp` and `satp`.
fn start_hart(state: &mut SystemState, hart_id: HartId, entry: VirtAddr, sp: VirtAddr, satp: u64) {
    let core = state.topology.core_of_hart(hart_id).map_or(hart_id.as_index(), CoreId::as_index);
    let mut ctx = state.core_ctx(core);
    enter_user_mode(&mut ctx, entry, satp);
    ctx.hart.regs.write(reg::REG_SP, sp.val());
}

/// Points `ctx`'s hart at `pc` in user mode under `satp`, with the whole
/// address space open to it, the counters readable, and the FPU and
/// vector unit on, as Linux leaves them for a process.
fn enter_user_mode(ctx: &mut CoreCtx<'_>, pc: VirtAddr, satp: u64) {
    ctx.hart.pc = pc.val();
    ctx.hart.privilege = PrivilegeMode::User;
    // A user-mode access no PMP entry matches is denied, so the firmware
    // that is not here would have opened the whole address space.
    ctx.csr_write(csr::PMPADDR0, PMP_NAPOT_EVERYTHING);
    ctx.csr_write(csr::PMPCFG0, PMP_RWX_NAPOT);
    ctx.csr_write(csr::SATP, satp);
    ctx.csr_write(csr::MCOUNTEREN, COUNTERS_ALL);
    ctx.csr_write(csr::SCOUNTEREN, COUNTERS_ALL);
    let mstatus = (ctx.hart.csrs.mstatus & !(csr::MSTATUS_FS | csr::MSTATUS_VS))
        | csr::MSTATUS_FS_INIT
        | csr::MSTATUS_VS_INIT;
    ctx.csr_write(csr::MSTATUS, mstatus);
}

/// Leaves `hart` waiting as after a `wfi` nothing can wake: it has no
/// thread, or its thread waits until another hart wakes it.
pub const fn park(hart: &mut Hart) {
    hart.wfi_waiting = true;
    hart.csrs.mie = 0;
}

/// The user-mode registers a thread starts with: a copy of the thread
/// that cloned it, as Linux's `clone` gives the child.
#[derive(Debug, Clone)]
pub struct UserContext {
    pc: VirtAddr,
    satp: u64,
    gprs: [u64; 32],
    fprs: [u64; 32],
    vprs: Vec<u8>,
    fflags: u64,
    frm: u64,
    vxrm: u64,
    vxsat: u64,
    vl: u64,
    vtype: u64,
}

impl UserContext {
    /// `hart`'s user registers and translation, to continue at `pc`.
    #[must_use]
    pub fn of(hart: &Hart, pc: VirtAddr) -> Self {
        let csrs = &hart.csrs;
        Self {
            pc,
            satp: csrs.satp,
            gprs: std::array::from_fn(|i| hart.regs.read(RegIdx::new(i as u8))),
            fprs: std::array::from_fn(|i| hart.regs.read_f(RegIdx::new(i as u8))),
            vprs: hart.regs.vpr().bytes().to_vec(),
            fflags: csrs.fflags,
            frm: csrs.frm,
            vxrm: csrs.vxrm,
            vxsat: csrs.vxsat,
            vl: csrs.vl,
            vtype: csrs.vtype,
        }
    }

    /// Sets integer register `reg` to `value`.
    pub const fn set(&mut self, reg: RegIdx, value: u64) {
        self.gprs[reg.as_usize()] = value;
    }
}

/// Starts a thread with `context` on `ctx`'s hart: user mode at its PC
/// under its translation, with its registers.
fn start_thread(ctx: &mut CoreCtx<'_>, context: &UserContext) {
    enter_user_mode(ctx, context.pc, context.satp);
    for (i, (&gpr, &fpr)) in (0u8..).zip(context.gprs.iter().zip(&context.fprs)) {
        ctx.hart.regs.write(RegIdx::new(i), gpr);
        ctx.hart.regs.write_f(RegIdx::new(i), fpr);
    }
    ctx.hart.regs.vpr_mut().set_bytes(&context.vprs);
    let csrs = &mut ctx.hart.csrs;
    csrs.fflags = context.fflags;
    csrs.frm = context.frm;
    csrs.vxrm = context.vxrm;
    csrs.vxsat = context.vxsat;
    csrs.vl = context.vl;
    csrs.vtype = context.vtype;
    csrs.vstart = 0;
}

/// Sets `ctx`'s parked hart going with what `wakeup` brings.
///
/// That is a new thread, or a waiting thread's syscall result; the hart
/// runs from its next cycle. The caller points fetch at the hart's PC and
/// syncs its pipeline's registers.
pub fn wake(ctx: &mut CoreCtx<'_>, wakeup: &Wakeup) {
    match &wakeup.kind {
        WakeupKind::Start(context) => start_thread(ctx, context),
        WakeupKind::Resume { a0 } => ctx.hart.regs.write(reg::REG_A0, *a0),
    }
    if wakeup.flush_translations {
        ctx.core.mmu.sfence_vma(&syscall::flush_all());
    }
    ctx.clear_reservation();
    ctx.hart.wfi_waiting = false;
}

const fn space_error(error: SpaceError) -> SimError {
    SimError::ProcessMemory(error)
}

/// One `PT_LOAD` segment.
struct Segment<'a> {
    vaddr: VirtAddr,
    memsz: u64,
    data: &'a [u8],
    permissions: Permissions,
}

/// What the loader needs from the ELF.
struct Image<'a> {
    entry: VirtAddr,
    segments: Vec<Segment<'a>>,
    phdr_vaddr: VirtAddr,
    phent: u64,
    phnum: u64,
}

impl<'a> Image<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, SimError> {
        let unsupported =
            |reason: &str| SimError::UnsupportedProgram { reason: reason.to_string() };
        let file = ElfFile64::<Endianness>::parse(data).map_err(|_| SimError::NotAnElf)?;
        let endian = file.endian();
        let header = file.elf_header();
        if header.e_machine(endian) != EM_RISCV {
            return Err(unsupported("not a RISC-V ELF"));
        }
        // A static PIE's addresses are relative to wherever it is loaded;
        // it goes where a static executable is linked, so both share one
        // layout.
        let bias = match header.e_type(endian) {
            ET_EXEC => 0,
            ET_DYN => PIE_BASE.val(),
            _ => return Err(unsupported("not an executable: link with -static or -static-pie")),
        };
        let phdrs = header.program_headers(endian, data).map_err(|_| SimError::NotAnElf)?;
        let mut segments = Vec::new();
        let mut phdr_vaddr = None;
        let e_phoff = header.e_phoff(endian);
        for phdr in phdrs {
            match phdr.p_type(endian) {
                PT_INTERP => {
                    return Err(unsupported(
                        "a dynamically linked program: link with -static or -static-pie",
                    ));
                }
                PT_PHDR => phdr_vaddr = Some(VirtAddr::new(phdr.p_vaddr(endian) + bias)),
                PT_LOAD => {
                    let offset = phdr.p_offset(endian);
                    let filesz = phdr.p_filesz(endian);
                    let vaddr = VirtAddr::new(phdr.p_vaddr(endian) + bias);
                    if phdr_vaddr.is_none() && offset <= e_phoff && e_phoff < offset + filesz {
                        phdr_vaddr = Some(vaddr.wrapping_add(e_phoff - offset));
                    }
                    let bytes = phdr.data(endian, data).map_err(|()| SimError::NotAnElf)?;
                    let flags = phdr.p_flags(endian);
                    segments.push(Segment {
                        vaddr,
                        memsz: phdr.p_memsz(endian),
                        data: bytes,
                        permissions: Permissions {
                            read: flags & PF_R != 0,
                            write: flags & PF_W != 0,
                            execute: flags & PF_X != 0,
                        },
                    });
                }
                _ => {}
            }
        }
        if segments.is_empty() {
            return Err(unsupported("no loadable segment"));
        }
        Ok(Self {
            entry: VirtAddr::new(header.e_entry(endian) + bias),
            segments,
            phdr_vaddr: phdr_vaddr.unwrap_or_default(),
            phent: u64::from(header.e_phentsize(endian)),
            phnum: u64::from(header.e_phnum(endian)),
        })
    }
}

/// Builds the initial stack as Linux's `create_elf_tables` does and
/// returns the stack pointer: the strings at the top, then the auxiliary
/// vector, `envp`, `argv` and `argc`, 16-byte aligned.
fn build_stack(
    process: &mut Process,
    memory: &mut GlobalMemory,
    image: &Image<'_>,
    args: &ProcessArgs,
) -> Result<VirtAddr, SpaceError> {
    let mut random_bytes = [0u8; 16];
    random_bytes[..8].copy_from_slice(&process.next_random().to_le_bytes());
    random_bytes[8..].copy_from_slice(&process.next_random().to_le_bytes());
    let space = &process.space;
    let mut sp = process.stack_top();
    let mut push_bytes = |sp: &mut VirtAddr, bytes: &[u8]| -> Result<VirtAddr, SpaceError> {
        *sp = sp.wrapping_sub(bytes.len() as u64);
        space.write(memory, *sp, bytes)?;
        Ok(*sp)
    };

    let mut argv_ptrs = Vec::with_capacity(args.argv.len());
    for arg in &args.argv {
        let mut bytes = arg.as_bytes().to_vec();
        bytes.push(0);
        argv_ptrs.push(push_bytes(&mut sp, &bytes)?);
    }
    let mut envp_ptrs = Vec::with_capacity(args.envp.len());
    for var in &args.envp {
        let mut bytes = var.as_bytes().to_vec();
        bytes.push(0);
        envp_ptrs.push(push_bytes(&mut sp, &bytes)?);
    }
    let execfn = argv_ptrs.first().copied().unwrap_or_default();
    let platform = push_bytes(&mut sp, b"riscv\0")?;
    let random = push_bytes(&mut sp, &random_bytes)?;

    let auxv = [
        (AT_PHDR, image.phdr_vaddr.val()),
        (AT_PHENT, image.phent),
        (AT_PHNUM, image.phnum),
        (AT_PAGESZ, PAGE_SIZE),
        (AT_BASE, 0),
        (AT_FLAGS, 0),
        (AT_ENTRY, image.entry.val()),
        (AT_UID, 0),
        (AT_EUID, 0),
        (AT_GID, 0),
        (AT_EGID, 0),
        (AT_PLATFORM, platform.val()),
        (AT_HWCAP, HWCAP_IMAFDC),
        (AT_CLKTCK, CLOCK_TICKS_PER_SECOND),
        (AT_SECURE, 0),
        (AT_RANDOM, random.val()),
        (AT_EXECFN, execfn.val()),
        (AT_NULL, 0),
    ];

    let words = 1 + (argv_ptrs.len() + 1) + (envp_ptrs.len() + 1) + auxv.len() * 2;
    sp = VirtAddr::new(sp.wrapping_sub((words as u64) * 8).val() & !0xF);
    let mut cursor = sp;
    let mut put = |cursor: &mut VirtAddr, value: u64| -> Result<(), SpaceError> {
        space.write_u64(memory, *cursor, value)?;
        *cursor = cursor.wrapping_add(8);
        Ok(())
    };
    put(&mut cursor, args.argv.len() as u64)?;
    for ptr in &argv_ptrs {
        put(&mut cursor, ptr.val())?;
    }
    put(&mut cursor, 0)?;
    for ptr in &envp_ptrs {
        put(&mut cursor, ptr.val())?;
    }
    put(&mut cursor, 0)?;
    for (key, value) in auxv {
        put(&mut cursor, key)?;
        put(&mut cursor, value)?;
    }
    Ok(sp)
}
