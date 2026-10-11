//! Loads a static Linux ELF as a user process.
//!
//! Its segments go into pages behind page tables the loader builds, the
//! initial stack is what Linux's ELF loader builds (argc, argv, envp and
//! the auxiliary vector), and hart 0 starts at the entry in user mode.

use object::Endianness;
use object::elf::{EM_RISCV, ET_DYN, ET_EXEC, PF_R, PF_W, PF_X, PT_INTERP, PT_LOAD, PT_PHDR};
use object::read::elf::{ElfFile64, FileHeader, ProgramHeader};

use super::{AddressSpace, PAGE_SIZE, Permissions, Process, SpaceError, page_ceil};
use crate::common::SimError;
use crate::isa::csr;
use crate::isa::privileged::{PagingMode, PrivilegeMode};
use crate::isa::reg;
use crate::soc::terminal::InputReader;
use crate::system::state::SystemState;

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
pub const PIE_BASE: u64 = 0x1_0000;
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

/// Loads `data`, a static riscv64 Linux ELF, as the process hart 0 runs
/// under `mode` paging.
///
/// # Errors
///
/// [`SimError::UnsupportedProgram`] for anything but a static RISC-V
/// executable, position-independent or not; [`SimError::ProcessMemory`]
/// when RAM cannot hold it.
pub fn load_process(
    state: &mut SystemState,
    data: &[u8],
    args: &ProcessArgs,
    mode: PagingMode,
    clock_hz: u64,
) -> Result<(), SimError> {
    if state.harts.len() != 1 {
        return Err(SimError::UnsupportedProgram {
            reason: "SE mode runs one hart; configure a single core".to_string(),
        });
    }
    if mode == PagingMode::Bare {
        return Err(SimError::UnsupportedProgram {
            reason: "SE mode needs a paging mode (Sv39, Sv48 or Sv57)".to_string(),
        });
    }
    let image = Image::parse(data)?;

    let ram_base = state.config.system.ram_base;
    let ram_end = ram_base + state.config.memory.ram_size as u64;
    let memory = &mut state.uncore.memory;
    let mut space = AddressSpace::new(mode, ram_base, ram_end, memory).map_err(space_error)?;

    let mut image_end = 0;
    for segment in &image.segments {
        let perms = segment.permissions;
        let mut page = segment.vaddr & !(PAGE_SIZE - 1);
        let end = segment.vaddr.checked_add(segment.memsz).ok_or_else(|| {
            SimError::UnsupportedProgram { reason: "a segment wraps the address space".to_string() }
        })?;
        while page < end {
            let merged = space.permissions(page).map_or(perms, |existing| existing.union(perms));
            space.map(memory, page, merged).map_err(space_error)?;
            page += PAGE_SIZE;
        }
        space.write(memory, segment.vaddr, segment.data).map_err(space_error)?;
        image_end = image_end.max(end);
    }

    let mut process =
        Process::new(space, page_ceil(image_end), clock_hz, state.uncore.terminal.clone());
    let stack_top = process.stack_top();
    let stack_bytes = process.stack_bytes();
    process
        .space
        .map_range(memory, stack_top - stack_bytes, stack_bytes, Permissions::RW)
        .map_err(space_error)?;
    let sp = build_stack(&mut process, memory, &image, args).map_err(space_error)?;

    let satp = process.space.satp();
    state.uncore.process = Some(process);
    state.uncore.read_console_through(InputReader::Proxy);
    let mut ctx = state.core_ctx(0);
    ctx.hart.pc = image.entry;
    ctx.hart.privilege = PrivilegeMode::User;
    ctx.hart.regs.write(reg::REG_SP, sp);
    // A user-mode access no PMP entry matches is denied, so the firmware
    // that is not here would have opened the whole address space.
    ctx.csr_write(csr::PMPADDR0, PMP_NAPOT_EVERYTHING);
    ctx.csr_write(csr::PMPCFG0, PMP_RWX_NAPOT);
    ctx.csr_write(csr::SATP, satp);
    ctx.csr_write(csr::MCOUNTEREN, COUNTERS_ALL);
    ctx.csr_write(csr::SCOUNTEREN, COUNTERS_ALL);
    // Linux turns the FPU and the vector unit on for a process; glibc's
    // string routines use the vector unit when the ISA has it.
    let mstatus = (ctx.hart.csrs.mstatus & !(csr::MSTATUS_FS | csr::MSTATUS_VS))
        | csr::MSTATUS_FS_INIT
        | csr::MSTATUS_VS_INIT;
    ctx.csr_write(csr::MSTATUS, mstatus);
    Ok(())
}

const fn space_error(error: SpaceError) -> SimError {
    SimError::ProcessMemory(error)
}

/// One `PT_LOAD` segment.
struct Segment<'a> {
    vaddr: u64,
    memsz: u64,
    data: &'a [u8],
    permissions: Permissions,
}

/// What the loader needs from the ELF.
struct Image<'a> {
    entry: u64,
    segments: Vec<Segment<'a>>,
    phdr_vaddr: u64,
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
            ET_DYN => PIE_BASE,
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
                PT_PHDR => phdr_vaddr = Some(phdr.p_vaddr(endian) + bias),
                PT_LOAD => {
                    let offset = phdr.p_offset(endian);
                    let filesz = phdr.p_filesz(endian);
                    let vaddr = phdr.p_vaddr(endian) + bias;
                    if phdr_vaddr.is_none() && offset <= e_phoff && e_phoff < offset + filesz {
                        phdr_vaddr = Some(vaddr + (e_phoff - offset));
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
            entry: header.e_entry(endian) + bias,
            segments,
            phdr_vaddr: phdr_vaddr.unwrap_or(0),
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
    memory: &mut crate::sim::memory::GlobalMemory,
    image: &Image<'_>,
    args: &ProcessArgs,
) -> Result<u64, SpaceError> {
    let mut random_bytes = [0u8; 16];
    random_bytes[..8].copy_from_slice(&process.next_random().to_le_bytes());
    random_bytes[8..].copy_from_slice(&process.next_random().to_le_bytes());
    let space = &process.space;
    let mut sp = process.stack_top();
    let mut push_bytes = |sp: &mut u64, bytes: &[u8]| -> Result<u64, SpaceError> {
        *sp -= bytes.len() as u64;
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
    let execfn = argv_ptrs.first().copied().unwrap_or(0);
    let platform = push_bytes(&mut sp, b"riscv\0")?;
    let random = push_bytes(&mut sp, &random_bytes)?;

    let auxv = [
        (AT_PHDR, image.phdr_vaddr),
        (AT_PHENT, image.phent),
        (AT_PHNUM, image.phnum),
        (AT_PAGESZ, PAGE_SIZE),
        (AT_BASE, 0),
        (AT_FLAGS, 0),
        (AT_ENTRY, image.entry),
        (AT_UID, 0),
        (AT_EUID, 0),
        (AT_GID, 0),
        (AT_EGID, 0),
        (AT_PLATFORM, platform),
        (AT_HWCAP, HWCAP_IMAFDC),
        (AT_CLKTCK, CLOCK_TICKS_PER_SECOND),
        (AT_SECURE, 0),
        (AT_RANDOM, random),
        (AT_EXECFN, execfn),
        (AT_NULL, 0),
    ];

    let words = 1 + (argv_ptrs.len() + 1) + (envp_ptrs.len() + 1) + auxv.len() * 2;
    sp = (sp - (words as u64) * 8) & !0xF;
    let mut cursor = sp;
    let mut put = |cursor: &mut u64, value: u64| -> Result<(), SpaceError> {
        space.write_u64(memory, *cursor, value)?;
        *cursor += 8;
        Ok(())
    };
    put(&mut cursor, args.argv.len() as u64)?;
    for ptr in &argv_ptrs {
        put(&mut cursor, *ptr)?;
    }
    put(&mut cursor, 0)?;
    for ptr in &envp_ptrs {
        put(&mut cursor, *ptr)?;
    }
    put(&mut cursor, 0)?;
    for (key, value) in auxv {
        put(&mut cursor, key)?;
        put(&mut cursor, value)?;
    }
    Ok(sp)
}
