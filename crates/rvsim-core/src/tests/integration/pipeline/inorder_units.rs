//! The in-order backend executes on real functional units.
//!
//! A result is forwarded to the next instruction as soon as its unit
//! produces it, a multi-cycle unit holds the dependent for its latency,
//! and a non-pipelined unit serialises the instructions that need it.

use crate::config::BackendKind;
use crate::config::Console;
use crate::config::{Config, PrefetcherKind};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{A1, A2, A3, A4, T0, T1, X0};
const CHAIN: u64 = 60;
const MUL_LATENCY: u64 = 3;
const DIV_LATENCY: u64 = 35;
const DIVIDES: u64 = 4;

/// Cycles until `retired` instructions have retired on a one-wide in-order core.
fn cycles_to_retire(program: &[u32], retired: u64) -> u64 {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::InOrder;
    config.pipeline.width = 1;
    config.pipeline.fu_config.int_mul_latency = MUL_LATENCY;
    config.pipeline.fu_config.int_div_latency = DIV_LATENCY;
    // Keep instruction supply off the critical path: the test times the backend.
    config.cache.l1_i.enabled = true;
    config.cache.l1_i.prefetcher = PrefetcherKind::NextLine;
    config.cache.l1_i.prefetch_degree = std::num::NonZero::new(4).unwrap();
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, program);
    let mut cycles = 0;
    while ctx.sim.state.harts[0].instructions_retired < retired && cycles < 100_000 {
        ctx.run(1);
        cycles += 1;
    }
    assert!(cycles < 100_000, "the program retired {retired} instructions");
    cycles
}

#[test]
fn a_dependent_add_chain_issues_one_per_cycle_through_the_bypass() {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> = (0..CHAIN).map(|_| i().addi(T0, T0, 1).build()).collect();
    program.push(i().jal(X0, 0).build());

    let cycles = cycles_to_retire(&program, CHAIN);

    assert!(cycles <= CHAIN + 30, "{CHAIN} dependent adds took {cycles} cycles");
}

#[test]
fn a_dependent_multiply_chain_waits_the_multiplier_latency() {
    let i = InstructionBuilder::new;
    let mut program = vec![i().addi(T0, X0, 1).build(), i().addi(T1, X0, 3).build()];
    program.extend((0..CHAIN).map(|_| i().mul(T0, T0, T1).build()));
    program.push(i().jal(X0, 0).build());

    let cycles = cycles_to_retire(&program, CHAIN + 2);

    assert!(
        cycles >= CHAIN * MUL_LATENCY,
        "{CHAIN} dependent multiplies took only {cycles} cycles"
    );
    assert!(
        cycles <= CHAIN * MUL_LATENCY + 30,
        "{CHAIN} dependent multiplies took {cycles} cycles"
    );
}

#[test]
fn independent_divides_serialise_on_the_non_pipelined_divider() {
    let i = InstructionBuilder::new;
    let program = vec![
        i().addi(T0, X0, 100).build(),
        i().addi(T1, X0, 7).build(),
        i().div(A1, T0, T1).build(),
        i().div(A2, T0, T1).build(),
        i().div(A3, T0, T1).build(),
        i().div(A4, T0, T1).build(),
        i().jal(X0, 0).build(),
    ];

    let cycles = cycles_to_retire(&program, 2 + DIVIDES);

    assert!(cycles >= DIVIDES * DIV_LATENCY, "{DIVIDES} divides took only {cycles} cycles");
}
