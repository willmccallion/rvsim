//! The in-order backend executes on real functional units.
//!
//! A result is forwarded to the next instruction as soon as its unit
//! produces it, a multi-cycle unit holds the dependent for its latency,
//! and a non-pipelined unit serialises the instructions that need it.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::{Config, Prefetcher};
use rvsim_core::core::pipeline::engine::BackendType;

const T0: u32 = 5;
const T1: u32 = 6;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const A4: u32 = 14;
const PROGRAM_BASE: u64 = 0x8000_0000;
const CHAIN: u64 = 60;
const MUL_LATENCY: u64 = 3;
const DIV_LATENCY: u64 = 35;
const DIVIDES: u64 = 4;

/// Cycles until `retired` instructions have retired on a one-wide in-order core.
fn cycles_to_retire(program: &[u32], retired: u64) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = BackendType::InOrder;
    config.pipeline.width = 1;
    config.pipeline.fu_config.int_mul_latency = MUL_LATENCY;
    config.pipeline.fu_config.int_div_latency = DIV_LATENCY;
    // Keep instruction supply off the critical path: the test times the backend.
    config.cache.l1_i.enabled = true;
    config.cache.l1_i.prefetcher = Prefetcher::NextLine;
    config.cache.l1_i.prefetch_degree = 4;
    config.system.uart_quiet = true;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, program);
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
    program.push(i().jal(0, 0).build());

    let cycles = cycles_to_retire(&program, CHAIN);

    assert!(cycles <= CHAIN + 30, "{CHAIN} dependent adds took {cycles} cycles");
}

#[test]
fn a_dependent_multiply_chain_waits_the_multiplier_latency() {
    let i = InstructionBuilder::new;
    let mut program = vec![i().addi(T0, 0, 1).build(), i().addi(T1, 0, 3).build()];
    program.extend((0..CHAIN).map(|_| i().mul(T0, T0, T1).build()));
    program.push(i().jal(0, 0).build());

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
        i().addi(T0, 0, 100).build(),
        i().addi(T1, 0, 7).build(),
        i().div(A1, T0, T1).build(),
        i().div(A2, T0, T1).build(),
        i().div(A3, T0, T1).build(),
        i().div(A4, T0, T1).build(),
        i().jal(0, 0).build(),
    ];

    let cycles = cycles_to_retire(&program, 2 + DIVIDES);

    assert!(cycles >= DIVIDES * DIV_LATENCY, "{DIVIDES} divides took only {cycles} cycles");
}
