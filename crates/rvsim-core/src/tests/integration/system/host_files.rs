//! gem5's readfile and writefile pseudo-operations move bytes between the
//! program's memory and the host's script file and output directory, in a
//! bare-metal image, a user program under SE mode and a paged supervisor
//! alike, reaching the memory through the hart's own translation.

use crate::isa::reg::RegIdx;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::harness::{build_simulator, store_words};
use crate::tests::support::open_pmp;
use crate::tests::support::regs::{A0, A1, A2, A3, T0, T1, T2, X0};
use std::fs;

use tempfile::TempDir;

use crate::Simulator;
use crate::common::{PhysAddr, VirtAddr};
use crate::config::{BackendKind, Config, Console};
use crate::isa::csr;
use crate::isa::op::pseudo::PseudoOp;
use crate::isa::privileged::PrivilegeMode;
use crate::system::process::errno::Errno;
use crate::system::simulator::{ProcessArgs, StopAt, StopReason};
use crate::tests::support::builder::instruction::{InstructionBuilder, JAL_SELF, M5_FAIL};
use crate::tests::support::harness::TestContext;

use super::se_mode::user_elf;

const SCRIPT: &[u8] = b"echo hello from the host\n";
const OUTPUT_NAME: &str = "out.bin";
const READ_FILE: u32 = PseudoOp::encode(0x50);
const WRITE_FILE: u32 = PseudoOp::encode(0x4f);
/// Where the buffer and the file name sit, past the code.
const BUFFER_OFFSET: usize = 0x100;
const BUFFER_BYTES: i32 = 64;
const NAME_OFFSET: usize = 0x180;
const BARE_BASE: u64 = 0x8000_0000;
const ET_EXEC: u16 = 2;
const SE_PROGRAM_BASE: u64 = 0x1_0080;
const RAM_SIZE: usize = 0x80_0000;
const ROOT_PPN: u64 = 0x8_0100;
const L1_PPN: u64 = 0x8_0101;
const L0_PPN: u64 = 0x8_0102;
const CODE_VA: u64 = 0x4002_0000;
const CODE_PA: u64 = 0x8036_0000;
const PTE_V: u64 = 1;
const PTE_RWX_AD: u64 = 0b1100_1111;
const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

/// The host's script file and output directory for one run.
struct Host {
    dir: TempDir,
}

impl Host {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        fs::write(dir.path().join("script"), SCRIPT).expect("the script is written");
        Self { dir }
    }

    fn attach(&self, sim: &mut Simulator) {
        let script = self.dir.path().join("script");
        sim.set_script_file(script.to_str().expect("utf-8 path")).expect("the script loads");
        let out = self.dir.path().join("out");
        sim.set_output_dir(out.to_str().expect("utf-8 path")).expect("the directory is made");
    }

    fn output(&self) -> Option<Vec<u8>> {
        fs::read(self.dir.path().join("out").join(OUTPUT_NAME)).ok()
    }
}

/// Pushes `auipc rd` and `addi rd` that leave the address `offset` bytes
/// from the program's start in `rd`.
fn address_of(program: &mut Vec<u32>, rd: RegIdx, offset: usize) {
    let i = InstructionBuilder::new;
    let here = program.len() * 4;
    program.push(i().auipc(rd, 0).build());
    program.push(i().addi(rd, rd, (offset - here) as i32).build());
}

/// Pushes `auipc rd, 1` and `addi rd` that leave the address of the page
/// after the program's in `rd`.
fn next_page_address(program: &mut Vec<u32>, rd: RegIdx) {
    let i = InstructionBuilder::new;
    let here = program.len() * 4;
    program.push(i().auipc(rd, 1).build());
    program.push(i().addi(rd, rd, -(here as i32)).build());
}

/// Lays a zeroed buffer and the output file's name past `code`.
fn with_data(mut code: Vec<u32>) -> Vec<u32> {
    assert!(code.len() * 4 <= BUFFER_OFFSET, "the code fits before the buffer");
    code.resize(NAME_OFFSET / 4, 0);
    let mut name = OUTPUT_NAME.as_bytes().to_vec();
    name.resize(8, 0);
    for word in name.chunks(4) {
        code.push(u32::from_le_bytes([word[0], word[1], word[2], word[3]]));
    }
    code
}

/// Reads the script into the buffer, writes the buffer to the output
/// file, reads the script again from where the first read ended, and
/// exits with the bytes written plus 100 times the bytes of the second
/// read.
fn exchange_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = Vec::new();
    address_of(&mut program, A0, BUFFER_OFFSET);
    program.push(i().addi(A1, X0, BUFFER_BYTES).build());
    program.push(i().addi(A2, X0, 0).build());
    program.push(READ_FILE);
    program.push(i().addi(T0, A0, 0).build());
    address_of(&mut program, A0, BUFFER_OFFSET);
    program.push(i().addi(A1, T0, 0).build());
    program.push(i().addi(A2, X0, 0).build());
    address_of(&mut program, A3, NAME_OFFSET);
    program.push(WRITE_FILE);
    program.push(i().addi(T1, A0, 0).build());
    address_of(&mut program, A0, BUFFER_OFFSET);
    program.push(i().addi(A1, X0, BUFFER_BYTES).build());
    program.push(i().addi(A2, T0, 0).build());
    program.push(READ_FILE);
    program.push(i().addi(T2, X0, 100).build());
    program.push(i().mul(A0, A0, T2).build());
    program.push(i().add(A1, T1, A0).build());
    program.push(M5_FAIL);
    program.push(JAL_SELF);
    with_data(program)
}

/// Reads the script into the buffer and exits with the buffer's first
/// byte, loaded by the instruction right behind the pseudo-operation.
fn read_then_load_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = Vec::new();
    address_of(&mut program, T0, BUFFER_OFFSET);
    program.push(i().addi(A0, T0, 0).build());
    program.push(i().addi(A1, X0, BUFFER_BYTES).build());
    program.push(i().addi(A2, X0, 0).build());
    program.push(READ_FILE);
    program.push(i().lbu(A1, T0, 0).build());
    program.push(M5_FAIL);
    program.push(JAL_SELF);
    with_data(program)
}

/// Asks readfile to fill the page after the program's, and exits with
/// what it returned.
fn read_into_the_next_page_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = Vec::new();
    next_page_address(&mut program, A0);
    program.push(i().addi(A1, X0, BUFFER_BYTES).build());
    program.push(i().addi(A2, X0, 0).build());
    program.push(READ_FILE);
    program.push(i().addi(A1, A0, 0).build());
    program.push(M5_FAIL);
    program.push(JAL_SELF);
    with_data(program)
}

fn config(backend: BackendKind) -> Config {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    config
}

/// Runs `program` as a bare-metal image on `backend` with `host`'s files
/// and returns the exit code.
fn run_bare(backend: BackendKind, host: &Host, program: &[u32]) -> (TestContext, Option<u64>) {
    let mut ctx = TestContext::with_fast_memory(&config(backend)).load_program(BARE_BASE, program);
    host.attach(&mut ctx.sim);
    ctx.run(20_000);
    let exit = ctx.sim.state.uncore.peek_exit();
    (ctx, exit)
}

/// Runs `program` as a user process on `backend` with `host`'s files and
/// returns the exit code.
fn run_se(backend: BackendKind, host: &Host, program: &[u32]) -> (Simulator, Option<u64>) {
    let mut sim = build_simulator(&config(backend));
    let args = ProcessArgs { argv: vec!["prog".to_string()], envp: vec![] };
    sim.load_process(&user_elf(program, ET_EXEC), &args).expect("the program loads");
    host.attach(&mut sim);
    let stop = sim
        .run_to(&StopAt { cycles: Some(50_000), ..StopAt::default() })
        .expect("the program runs");
    let exit = match stop {
        StopReason::Exited(code) => Some(code),
        _ => None,
    };
    (sim, exit)
}

fn write_pte(ctx: &mut TestContext, table_ppn: u64, va: u64, level: u32, pte: u64) {
    let index = (va >> (12 + 9 * level)) & 0x1ff;
    ctx.sim.probe_mem_store(PhysAddr::new((table_ppn << 12) | (index * 8)), pte, ProbeWidth::B8);
}

/// Runs `program` in supervisor mode on `backend`, its one page mapped
/// under Sv39 at `CODE_VA`, with `host`'s files, and returns the exit
/// code.
fn run_paged(backend: BackendKind, host: &Host, program: &[u32]) -> (TestContext, Option<u64>) {
    let mut ctx = TestContext::with_fast_memory(&config(backend)).with_memory(RAM_SIZE, BARE_BASE);
    store_words(&mut ctx.sim, CODE_PA, program);
    write_pte(&mut ctx, ROOT_PPN, CODE_VA, 2, (L1_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L1_PPN, CODE_VA, 1, (L0_PPN << 10) | PTE_V);
    write_pte(&mut ctx, L0_PPN, CODE_VA, 0, ((CODE_PA >> 12) << 10) | PTE_RWX_AD);
    {
        let hart = &mut ctx.sim.state.harts[0];
        hart.csrs.satp = (csr::SATP_MODE_SV39 << 60) | ROOT_PPN;
        hart.csrs.mtvec = CODE_PA;
        hart.privilege = PrivilegeMode::Supervisor;
        open_pmp(&mut hart.pmp);
        hart.pc = VirtAddr::new(CODE_VA);
    }
    ctx.sim.sync_arch_regs();
    host.attach(&mut ctx.sim);
    ctx.run(20_000);
    let exit = ctx.sim.state.uncore.peek_exit();
    (ctx, exit)
}

fn script_length() -> u64 {
    SCRIPT.len() as u64
}

#[test]
fn a_bare_metal_image_exchanges_files_with_the_host() {
    for backend in BACKENDS {
        let host = Host::new();

        let (mut ctx, exit) = run_bare(backend, &host, &exchange_program());

        assert_eq!(exit, Some(script_length()), "{backend:?}: the whole script, then its end");
        assert_eq!(host.output().as_deref(), Some(SCRIPT), "{backend:?}");
        let first_word =
            ctx.sim.probe_mem_load(PhysAddr::new(BARE_BASE + BUFFER_OFFSET as u64), ProbeWidth::B4);
        assert_eq!(
            first_word.to_le_bytes()[..4],
            SCRIPT[..4],
            "{backend:?}: the buffer was filled"
        );
    }
}

#[test]
fn a_load_behind_readfile_sees_what_it_wrote() {
    for backend in BACKENDS {
        let host = Host::new();

        let (_, exit) = run_bare(backend, &host, &read_then_load_program());

        assert_eq!(exit, Some(u64::from(SCRIPT[0])), "{backend:?}");
    }
}

#[test]
fn a_user_program_exchanges_files_with_the_host() {
    for backend in BACKENDS {
        let host = Host::new();

        let (mut sim, exit) = run_se(backend, &host, &exchange_program());

        assert_eq!(exit, Some(script_length()), "{backend:?}");
        assert_eq!(host.output().as_deref(), Some(SCRIPT), "{backend:?}");
        let buffer = sim
            .translate_now(0, crate::common::VirtAddr::new(SE_PROGRAM_BASE + BUFFER_OFFSET as u64))
            .expect("the buffer is mapped");
        let first_word = sim.probe_mem_load(buffer, ProbeWidth::B4);
        assert_eq!(
            first_word.to_le_bytes()[..4],
            SCRIPT[..4],
            "{backend:?}: the buffer was filled"
        );
    }
}

#[test]
fn a_paged_supervisor_exchanges_files_through_its_page_tables() {
    for backend in BACKENDS {
        let host = Host::new();

        let (mut ctx, exit) = run_paged(backend, &host, &exchange_program());

        assert_eq!(exit, Some(script_length()), "{backend:?}");
        assert_eq!(host.output().as_deref(), Some(SCRIPT), "{backend:?}");
        let first_word =
            ctx.sim.probe_mem_load(PhysAddr::new(CODE_PA + BUFFER_OFFSET as u64), ProbeWidth::B4);
        assert_eq!(
            first_word.to_le_bytes()[..4],
            SCRIPT[..4],
            "{backend:?}: the buffer was filled"
        );
    }
}

#[test]
fn readfile_into_an_unmapped_buffer_fails_with_efault() {
    for backend in BACKENDS {
        let host = Host::new();

        let (_, exit) = run_paged(backend, &host, &read_into_the_next_page_program());

        assert_eq!(exit, Some(Errno::EFAULT.return_value()), "{backend:?}");
    }
}

#[test]
fn without_a_script_readfile_reads_nothing() {
    for backend in BACKENDS {
        let host = Host::new();
        let mut ctx = TestContext::with_fast_memory(&config(backend))
            .load_program(BARE_BASE, &exchange_program());
        let out = host.dir.path().join("out");
        ctx.sim.set_output_dir(out.to_str().expect("utf-8 path")).expect("the directory is made");

        ctx.run(20_000);

        assert_eq!(
            ctx.sim.state.uncore.peek_exit(),
            Some(0),
            "{backend:?}: nothing read, nothing written"
        );
        assert_eq!(host.output().as_deref(), Some(&b""[..]), "{backend:?}: an empty file");
    }
}
