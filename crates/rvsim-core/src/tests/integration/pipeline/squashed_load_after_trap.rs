//! A load still waiting on memory when a trap flushes the pipeline never
//! completes: its answer must not reach the physical register it had,
//! which the flush freed and the trap handler may have reused.

use crate::Simulator;
use crate::common::PhysAddr;
use crate::config::{BackendKind, Config, Console, MemoryControllerKind};
use crate::isa::csr::{MSTATUS_FS_INIT, MSTATUS_VS_INIT};
use crate::isa::reg::RegIdx;
use crate::soc::exit::ExitSignal;
use crate::system::SystemState;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::harness::valid;
use crate::tests::support::regs::{
    A0, A5, T0, T2, X0, X6, X11, X12, X13, X14, X18, X19, X20, X21, X22, X23, X24, X25, X26, X28,
    X29, X30, X31,
};

const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA: u64 = PROGRAM_BASE + 0x400;
const LOADED: u64 = 0x1234_5678_9ABC_DEF0;
const LATENCY: u64 = 600;
/// SYSTEM with funct3=4, which the hart does not implement.
const ILLEGAL: u32 = 0x0000_4073;
const HANDLER_INDEX: usize = 7;
/// What the handler writes in the I-cache line it shares with the load.
const WRITTEN: [RegIdx; 9] = [X6, X11, X12, X13, X14, X28, X29, X30, X31];
/// Where the handler copies each, once its next line has arrived.
const COPIES: [RegIdx; 9] = [X18, X19, X20, X21, X22, X23, X24, X25, X26];
const BASE_VALUE: i32 = 100;

/// Divides hold an illegal instruction from commit while the load after
/// it misses to memory. The handler, in the same I-cache line, writes nine
/// registers; the line after it, which arrives only after the load's
/// answer, copies them.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(T0, X0, 1).build(),
        i().div(T2, T0, T0).build(),
        i().div(T2, T2, T0).build(),
        ILLEGAL,
        i().ld(A5, A0, 0).build(),
    ];
    assert_eq!(program.len(), HANDLER_INDEX);
    program.extend(WRITTEN.iter().zip(BASE_VALUE..).map(|(&r, v)| i().addi(r, X0, v).build()));
    program.extend(WRITTEN.iter().zip(COPIES).map(|(&r, copy)| i().addi(copy, r, 0).build()));
    program.push(i().jal(X0, 0).build());
    program
}

/// A one-core machine with L1 caches in front of a memory answering in
/// `latency` cycles, and few rename registers to spare, so the register
/// the flush frees is soon reused.
fn context(backend: BackendKind, latency: u64) -> TestContext {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.prf_gpr_size = 40;
    config.pipeline.prf_fpr_size = 32;
    config.system.console = Console::Quiet;
    config.cache.l1_i.enabled = true;
    config.cache.l1_d.enabled = true;
    config.memory.controller = MemoryControllerKind::Simple;
    config.memory.simple_latency = latency;
    config.system.bus_latency = 0;
    let mut state = SystemState::new(&valid(&config), None, ExitSignal::new())
        .expect("the test machine builds");
    for hart in &mut state.harts {
        hart.csrs.mstatus |= MSTATUS_FS_INIT | MSTATUS_VS_INIT;
    }
    TestContext { sim: Simulator::new(state) }.load_program(PROGRAM_BASE, &program())
}

/// The handler's copies once it has finished.
fn copies(backend: BackendKind) -> Vec<u64> {
    let mut ctx = context(backend, LATENCY);
    ctx.sim.probe_mem_store(PhysAddr::new(DATA), LOADED, ProbeWidth::B8);
    ctx.sim.state.harts[0].csrs.mtvec = PROGRAM_BASE + 4 * HANDLER_INDEX as u64;
    ctx.sim.sync_arch_regs();

    ctx.run(8 * LATENCY);

    COPIES.iter().map(|&r| ctx.get_reg(r)).collect()
}

fn written() -> Vec<u64> {
    (BASE_VALUE..).take(WRITTEN.len()).map(|v| v as u64).collect()
}

#[test]
fn a_load_squashed_by_a_trap_does_not_write_the_handlers_registers_o3() {
    let copies = copies(BackendKind::OutOfOrder);

    assert_eq!(copies, written(), "what the handler copied");
}

#[test]
fn a_load_squashed_by_a_trap_does_not_write_the_handlers_registers_inorder() {
    let copies = copies(BackendKind::InOrder);

    assert_eq!(copies, written(), "what the handler copied");
}
