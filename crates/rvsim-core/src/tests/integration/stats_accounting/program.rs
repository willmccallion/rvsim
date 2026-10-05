//! Loading and running the small programs the accounting checks count.

use crate::Simulator;
use crate::common::PhysAddr;
use crate::config::{BackendKind, Config};
use crate::isa::csr::{MSTATUS_FS, MSTATUS_FS_INIT, MSTATUS_VS, MSTATUS_VS_INIT};
use crate::system::SystemState;
use crate::system::simulator::{StopAt, StopReason};
use crate::tests::support::builder::instruction::{ECALL, InstructionBuilder, MRET};
use crate::tests::support::harness::TestContext;

pub const PROGRAM_BASE: u64 = 0x8000_0000;
pub const HANDLER: u64 = PROGRAM_BASE + 0x400;
pub const DATA: u64 = PROGRAM_BASE + 0x1000;
pub const T0: u32 = 5;
pub const T1: u32 = 6;
pub const T2: u32 = 7;
pub const A0: u32 = 10;
pub const A1: u32 = 11;
pub const A2: u32 = 12;
pub const A7: u32 = 17;
pub const MEPC: u32 = 0x341;
pub const MTVEC: u32 = 0x305;
pub const MHARTID: u32 = 0xF14;
const SYS_EXIT: i32 = 93;

pub const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

pub fn config(backend: BackendKind) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = crate::config::Console::Quiet;
    config
}

pub fn store_words(ctx: &mut TestContext, at: u64, words: &[u32]) {
    for (n, word) in words.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(at + 4 * n as u64), u64::from(*word), 4);
    }
}

/// Loads `program`, and `handler` at `HANDLER` when given, with the FP and
/// vector units on and `a1`/`a2` pointing at data; the guest's exit ecall
/// ends the run. The test harness's memory answers in one cycle.
pub fn system(backend: BackendKind, program: &[u32], handler: &[u32]) -> TestContext {
    system_with(&config(backend), program, handler)
}

/// [`system`] with `config`.
pub fn system_with(config: &Config, program: &[u32], handler: &[u32]) -> TestContext {
    prepare(TestContext::new_with_config(config), program, handler)
}

/// [`system_with`] keeping `config`'s memory and bus latencies, which the
/// test harness would cut to one cycle and zero.
pub fn system_with_configured_latency(
    config: &Config,
    program: &[u32],
    handler: &[u32],
) -> TestContext {
    let ctx = TestContext { sim: Simulator::new(SystemState::build(config, "")) };
    prepare(ctx, program, handler)
}

fn prepare(ctx: TestContext, program: &[u32], handler: &[u32]) -> TestContext {
    let mut ctx = ctx.load_program(PROGRAM_BASE, program);
    store_words(&mut ctx, HANDLER, handler);
    let csrs = &mut ctx.sim.state.harts[0].csrs;
    csrs.mstatus = (csrs.mstatus & !(MSTATUS_FS | MSTATUS_VS)) | MSTATUS_FS_INIT | MSTATUS_VS_INIT;
    if !handler.is_empty() {
        csrs.mtvec = HANDLER;
    }
    ctx.set_reg(A1 as usize, DATA);
    ctx.set_reg(A2 as usize, DATA + 0x100);
    ctx.sim.set_direct_mode(true);
    ctx.sim.sync_arch_regs();
    ctx
}

/// `program` followed by a jump to itself, and the jump's address.
pub fn ending_in_spin(mut program: Vec<u32>) -> (Vec<u32>, u64) {
    let end = PROGRAM_BASE + 4 * program.len() as u64;
    program.push(InstructionBuilder::new().jal(0, 0).build());
    (program, end)
}

/// Runs until the hart is about to run `end`.
pub fn run_to_pc(ctx: &mut TestContext, end: u64, context: &str) {
    let stop = StopAt { pcs: vec![end], cycles: Some(100_000), ..StopAt::default() };
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
    [i().addi(A0, 0, 0).build(), i().addi(A7, 0, SYS_EXIT).build(), ECALL]
}

/// Takes three ecalls into a handler that steps over each, then exits.
pub fn three_handled_ecalls() -> (Vec<u32>, Vec<u32>) {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T1, 0, 3).build(),
        ECALL,
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -8).build(),
        i().csrrw(0, MTVEC, 0).build(),
    ];
    program.extend(exit_sequence());
    let handler = vec![
        i().csrrs(T0, MEPC, 0).build(),
        i().addi(T0, T0, 4).build(),
        i().csrrw(0, MEPC, T0).build(),
        MRET,
    ];
    (program, handler)
}
