//! Loading and running the small programs the accounting checks count.

use crate::Simulator;
use crate::common::{PhysAddr, VirtAddr};
use crate::config::Console;
use crate::config::{BackendKind, Config};
use crate::isa::csr::{
    MSTATUS_FS, MSTATUS_FS_INIT, MSTATUS_MPP, MSTATUS_VS, MSTATUS_VS_INIT, SATP_MODE_SV39,
};
use crate::isa::privileged::PrivilegeMode;
use crate::system::SystemState;
use crate::system::simulator::{ProbeWidth, StopAt, StopReason};
use crate::tests::support::builder::instruction::{ECALL, InstructionBuilder, M5_FAIL, MRET};
use crate::tests::support::harness::{PROGRAM_BASE, TestContext, store_words};
use crate::tests::support::open_pmp;
use crate::tests::support::regs::{A0, A1, A2, T0, T1, X0};

pub const HANDLER: u64 = PROGRAM_BASE + 0x400;
pub const DATA: u64 = PROGRAM_BASE + 0x1000;
pub const MEPC: u32 = 0x341;
pub const MTVEC: u32 = 0x305;
pub const MHARTID: u32 = 0xF14;

pub const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

pub fn config(backend: BackendKind) -> Config {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    config
}

/// Loads `program`, and `handler` at `HANDLER` when given, with the FP and
/// vector units on and `a1`/`a2` pointing at data; the guest's exit ecall
/// ends the run. The test harness's memory answers in one cycle.
pub fn system(backend: BackendKind, program: &[u32], handler: &[u32]) -> TestContext {
    system_with(&config(backend), program, handler)
}

/// [`system`] with `config`.
pub fn system_with(config: &Config, program: &[u32], handler: &[u32]) -> TestContext {
    prepare(TestContext::with_fast_memory(config), program, handler)
}

/// [`system_with`] keeping `config`'s memory and bus latencies, which the
/// test harness would cut to one cycle and zero.
pub fn system_with_configured_latency(
    config: &Config,
    program: &[u32],
    handler: &[u32],
) -> TestContext {
    let ctx = TestContext { sim: Simulator::new(SystemState::build(config)) };
    prepare(ctx, program, handler)
}

fn prepare(ctx: TestContext, program: &[u32], handler: &[u32]) -> TestContext {
    let mut ctx = ctx.load_program(PROGRAM_BASE, program);
    store_words(&mut ctx.sim, HANDLER, handler);
    let csrs = &mut ctx.sim.state.harts[0].csrs;
    csrs.mstatus = (csrs.mstatus & !(MSTATUS_FS | MSTATUS_VS)) | MSTATUS_FS_INIT | MSTATUS_VS_INIT;
    if !handler.is_empty() {
        csrs.mtvec = HANDLER;
    }
    ctx.set_reg(A1, DATA);
    ctx.set_reg(A2, DATA + 0x100);
    ctx.sim.sync_arch_regs();
    ctx
}

/// Where the one page table of [`supervisor_system`] lives.
pub const PAGE_TABLE: u64 = PROGRAM_BASE + 0x3000;
/// The page table's entry for the gigapage holding the program and data.
pub const GIGAPAGE_PTE: u64 = PAGE_TABLE + 8 * (PROGRAM_BASE >> 30);
/// A valid, readable, writable, executable leaf with A and D set.
const LEAF_RWXAD: u64 = 0xCF;

/// Whether [`supervisor_system`] turns on address translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Translation {
    /// Sv39, through one gigapage mapping everything to itself.
    Sv39,
    /// None.
    Bare,
}

/// Instructions [`supervisor_system`] runs before the supervisor's body.
pub const SUPERVISOR_ENTRY: usize = 4;

/// `config` running `body` in supervisor mode, entered by an `mret`, under
/// `translation`; and the address of the spin it ends in.
pub fn supervisor_system(
    config: &Config,
    body: Vec<u32>,
    translation: Translation,
) -> (TestContext, u64) {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(T1, 0).build(),
        i().addi(T0, T1, 16).build(),
        i().csrrw(X0, MEPC, T0).build(),
        MRET,
    ];
    assert_eq!(program.len(), SUPERVISOR_ENTRY);
    program.extend(body);
    let (program, end) = ending_in_spin(program);
    let mut ctx = system_with(config, &program, &[]);
    let pte = ((PROGRAM_BASE >> 12) << 10) | LEAF_RWXAD;
    ctx.sim.probe_mem_store(PhysAddr::new(GIGAPAGE_PTE), pte, ProbeWidth::B8);
    let hart = &mut ctx.sim.state.harts[0];
    let supervisor = u64::from(PrivilegeMode::Supervisor.to_u8()) << 11;
    hart.csrs.mstatus = (hart.csrs.mstatus & !MSTATUS_MPP) | supervisor;
    if translation == Translation::Sv39 {
        hart.csrs.satp = (SATP_MODE_SV39 << 60) | (PAGE_TABLE >> 12);
    }
    open_pmp(&mut hart.pmp);
    (ctx, end)
}

/// `program` followed by a jump to itself, and the jump's address.
pub fn ending_in_spin(mut program: Vec<u32>) -> (Vec<u32>, u64) {
    let end = PROGRAM_BASE + 4 * program.len() as u64;
    program.push(InstructionBuilder::new().jal(X0, 0).build());
    (program, end)
}

/// Runs until the hart is about to run `end`.
pub fn run_to_pc(ctx: &mut TestContext, end: u64, context: &str) {
    let stop = StopAt { pcs: vec![VirtAddr::new(end)], cycles: Some(100_000), ..StopAt::default() };
    let reason = ctx.sim.run_to(&stop).expect("the run ticks");
    assert_eq!(reason, StopReason::Pc { hart: 0 }, "{context}");
}

pub fn run_to_exit(ctx: &mut TestContext, context: &str) {
    let reason = ctx
        .sim
        .run_to(&StopAt { cycles: Some(100_000), ..StopAt::default() })
        .expect("the run ticks");
    assert_eq!(reason, StopReason::Exited(0), "{context}");
}

/// `exit(0)`. The ecall traps, so it does not retire.
pub fn exit_sequence() -> [u32; 3] {
    let i = InstructionBuilder::new;
    [i().addi(A0, X0, 0).build(), i().addi(A1, X0, 0).build(), M5_FAIL]
}

/// Takes three ecalls into a handler that steps over each, then exits.
pub fn three_handled_ecalls() -> (Vec<u32>, Vec<u32>) {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T1, X0, 3).build(),
        ECALL,
        i().addi(T1, T1, -1).build(),
        i().bne(T1, X0, -8).build(),
        i().csrrw(X0, MTVEC, X0).build(),
    ];
    program.extend(exit_sequence());
    let handler = vec![
        i().csrrs(T0, MEPC, X0).build(),
        i().addi(T0, T0, 4).build(),
        i().csrrw(X0, MEPC, T0).build(),
        MRET,
    ];
    (program, handler)
}
