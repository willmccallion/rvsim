//! The hardware performance counters `mcycle` and `minstret` and their
//! `cycle` / `instret` aliases: they count on their own, can be written
//! without touching the simulator, and stop when `mcountinhibit` says so.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::core::arch::csr;

const T0: u32 = 5;
const PROGRAM_BASE: u64 = 0x8000_0000;
const NOPS: u64 = 5;
const REAL_INSTRUCTIONS: u64 = 3;

/// Five NOPs, three real instructions, then a spin.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> = (0..NOPS).map(|_| i().nop().build()).collect();
    program.extend([
        i().addi(T0, 0, 1).build(),
        i().addi(T0, T0, 2).build(),
        i().addi(T0, T0, 3).build(),
        i().jal(0, 0).build(),
    ]);
    program
}

fn context() -> TestContext {
    TestContext::new().load_program(PROGRAM_BASE, &program())
}

#[test]
fn mcycle_counts_the_cycles_the_hart_runs() {
    let mut ctx = context();

    ctx.run(40);

    assert_eq!(ctx.sim.state.core_ctx(0).csr_read(csr::MCYCLE), 40);
    assert_eq!(ctx.sim.state.core_ctx(0).csr_read(csr::CYCLE), 40);
}

#[test]
fn writing_mcycle_leaves_the_simulator_clock_alone() {
    let mut ctx = context();
    ctx.run(10);
    let clock = ctx.sim.state.cycle;

    ctx.sim.state.core_ctx(0).csr_write(csr::MCYCLE, 1_000);

    assert_eq!(ctx.sim.state.cycle, clock);
    ctx.run(5);
    assert_eq!(ctx.sim.state.cycle, clock + 5);
    assert_eq!(ctx.sim.state.core_ctx(0).csr_read(csr::MCYCLE), 1_005);
}

#[test]
fn mcountinhibit_cy_freezes_mcycle_only() {
    let mut ctx = context();
    ctx.run(10);

    ctx.sim.state.core_ctx(0).csr_write(csr::MCOUNTINHIBIT, csr::MCOUNTINHIBIT_CY);
    ctx.run(20);

    assert_eq!(ctx.sim.state.core_ctx(0).csr_read(csr::MCYCLE), 10);
    assert_eq!(ctx.sim.state.core_ctx(0).csr_read(csr::MCOUNTINHIBIT), csr::MCOUNTINHIBIT_CY);
    assert!(ctx.sim.state.core_ctx(0).csr_read(csr::MINSTRET) > 0, "instructions still count");
}

#[test]
fn only_the_cy_and_ir_bits_of_mcountinhibit_are_writable() {
    let mut ctx = context();

    ctx.sim.state.core_ctx(0).csr_write(csr::MCOUNTINHIBIT, u64::MAX);

    assert_eq!(
        ctx.sim.state.core_ctx(0).csr_read(csr::MCOUNTINHIBIT),
        csr::MCOUNTINHIBIT_CY | csr::MCOUNTINHIBIT_IR
    );
}

#[test]
fn minstret_counts_every_retired_instruction_including_nops() {
    let mut ctx = context();

    ctx.run(200);

    let minstret = ctx.sim.state.core_ctx(0).csr_read(csr::MINSTRET);
    assert!(minstret >= NOPS + REAL_INSTRUCTIONS, "the NOPs and the program retired");
    assert_eq!(minstret, ctx.sim.state.harts[0].instructions_retired);
    assert_eq!(ctx.sim.state.core_ctx(0).csr_read(csr::INSTRET), minstret);
}

#[test]
fn writing_minstret_sets_the_counter_without_touching_the_stats() {
    let mut ctx = context();
    ctx.run(200);
    let retired = ctx.sim.state.harts[0].instructions_retired;

    ctx.sim.state.core_ctx(0).csr_write(csr::MINSTRET, 7);

    assert_eq!(ctx.sim.state.core_ctx(0).csr_read(csr::MINSTRET), 7);
    assert_eq!(ctx.sim.state.harts[0].instructions_retired, retired);
}

#[test]
fn mcountinhibit_ir_freezes_minstret() {
    let mut ctx = context();
    ctx.run(50);
    let frozen = ctx.sim.state.core_ctx(0).csr_read(csr::MINSTRET);

    ctx.sim.state.core_ctx(0).csr_write(csr::MCOUNTINHIBIT, csr::MCOUNTINHIBIT_IR);
    ctx.run(50);

    assert_eq!(ctx.sim.state.core_ctx(0).csr_read(csr::MINSTRET), frozen);
    assert_eq!(ctx.sim.state.core_ctx(0).csr_read(csr::MCYCLE), 100, "cycles still count");
}

#[test]
fn the_time_csr_reads_the_clint_counter() {
    let mut ctx = context();

    ctx.run(37);

    let time = ctx.sim.state.core_ctx(0).csr_read(csr::TIME);
    assert_eq!(time, ctx.sim.state.bus.mtime());
    assert!(time > 0, "the CLINT has ticked");
}
