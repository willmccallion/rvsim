//! Taking a trap costs the configured trap latency, and an interrupt lets
//! everything already fetched retire before it is taken.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const T3: u32 = 28;
const T4: u32 = 29;
const T6: u32 = 31;
const MSTATUS: u32 = 0x300;
const MIE: u32 = 0x304;
const TIME: u32 = 0xC01;
const MIE_MTIE: i32 = 1 << 7;
const MSTATUS_MIE: i32 = 1 << 3;
const MACHINE_TIMER_INTERRUPT: u64 = (1 << 63) | 7;
const DIVS: usize = 8;
const ECALL: u32 = 0x0000_0073;

/// Spins.
fn handler() -> Vec<u32> {
    vec![InstructionBuilder::new().jal(0, 0).build()]
}

/// Arms the machine timer far enough ahead that the refetch after the
/// mstatus write has landed, then runs a chain of divides long enough for
/// it to fire while they are in flight, then a marker.
fn timer_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().lui(T0, 0x02004).build(),
        i().csrrs(T1, TIME, 0).build(),
        i().addi(T1, T1, 16).build(),
        i().sd(T0, T1, 0).build(),
        i().addi(T2, 0, MIE_MTIE).build(),
        i().csrrs(0, MIE, T2).build(),
        i().addi(T2, 0, MSTATUS_MIE).build(),
        i().csrrs(0, MSTATUS, T2).build(),
        i().addi(T4, 0, 1).build(),
    ];
    program.extend((0..DIVS).map(|_| i().div(T3, T3, T4).build()));
    program.push(i().addi(T6, 0, 1).build());
    program.push(i().jal(0, 0).build());
    program
}

fn spin_pc(program: &[u32]) -> u64 {
    PROGRAM_BASE + (program.len() as u64 - 1) * 4
}

struct TrapTaken {
    cycle: u64,
    mepc: u64,
    mcause: u64,
    marker: u64,
}

fn run_until_trap(backend: BackendType, trap_latency: u64, program: &[u32]) -> TrapTaken {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.pipeline.trap_latency = trap_latency;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, program);
    for (i, word) in handler().iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(HANDLER + (i as u64) * 4), u64::from(*word), 4);
    }
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    let mut cycle = 0;
    while ctx.sim.state.harts[0].csrs.mcause == 0 {
        ctx.run(1);
        cycle += 1;
        assert!(cycle < 5000, "{backend:?}: no trap was taken");
    }
    let csrs = &ctx.sim.state.harts[0].csrs;
    TrapTaken { cycle, mepc: csrs.mepc, mcause: csrs.mcause, marker: ctx.get_reg(T6 as usize) }
}

fn check_exception_latency(backend: BackendType) {
    let program = [ECALL, InstructionBuilder::new().jal(0, 0).build()];

    let at_once = run_until_trap(backend, 0, &program);
    let delayed = run_until_trap(backend, 13, &program);

    assert_eq!(at_once.mcause, 11, "{backend:?}: machine ecall");
    assert_eq!(at_once.mepc, PROGRAM_BASE);
    assert_eq!(
        delayed.cycle - at_once.cycle,
        13,
        "{backend:?}: the trap was taken 13 cycles later"
    );
}

fn check_interrupt_drains_and_waits(backend: BackendType) {
    let program = timer_program();

    let at_once = run_until_trap(backend, 0, &program);
    let delayed = run_until_trap(backend, 13, &program);

    assert_eq!(at_once.mcause, MACHINE_TIMER_INTERRUPT, "{backend:?}: machine timer interrupt");
    assert_eq!(at_once.marker, 1, "{backend:?}: everything fetched before the interrupt retired");
    assert_eq!(at_once.mepc, spin_pc(&program), "{backend:?}: the interrupt returns to the spin");
    assert_eq!(
        delayed.cycle - at_once.cycle,
        13,
        "{backend:?}: the trap was taken 13 cycles later"
    );
}

#[test]
fn an_exception_reaches_its_handler_after_the_trap_latency_o3() {
    check_exception_latency(BackendType::OutOfOrder);
}

#[test]
fn an_exception_reaches_its_handler_after_the_trap_latency_inorder() {
    check_exception_latency(BackendType::InOrder);
}

#[test]
fn an_interrupt_waits_for_the_fetched_instructions_then_the_trap_latency_o3() {
    check_interrupt_drains_and_waits(BackendType::OutOfOrder);
}

#[test]
fn an_interrupt_waits_for_the_fetched_instructions_then_the_trap_latency_inorder() {
    check_interrupt_drains_and_waits(BackendType::InOrder);
}
