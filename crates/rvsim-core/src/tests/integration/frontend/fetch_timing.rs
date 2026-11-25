//! What the front end takes to fetch a loop.
//!
//! Fetch reads one aligned block per cycle, so a loop body that straddles
//! two blocks takes two cycles to fetch. After a taken branch the BTB
//! predicts, fetch idles for the bubbles of the BTB that held it, the
//! micro-BTB's when it holds the branch. Each figure is the difference
//! between two loop lengths, so warm-up cancels.

use crate::config::{BackendKind, BranchPredictorKind, Config, MicroBtbConfig};
use crate::isa::reg::RegIdx;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{X0, X5};

const DONE_REG: RegIdx = RegIdx::new(31);
const DONE: i32 = 7;
/// The two loop lengths whose difference is measured.
const ITERATIONS: (i32, i32) = (100, 300);

/// A four-wide out-of-order core with an I-cache.
fn core() -> Config {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.width = 4;
    // One that learns the loop branch: the default predicts not taken.
    config.pipeline.branch_predictor = BranchPredictorKind::GShare;
    config.cache.l1_i.enabled = true;
    config
}

/// A loop of `iterations` iterations, each `body` then a decrement and a
/// branch back, after `padding` nops; then the done marker.
fn counted_loop(iterations: i32, padding: usize, body: &[u32]) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().addi(X5, X0, iterations).build()];
    program.extend(std::iter::repeat_n(i().nop().build(), padding));
    program.extend(body);
    let back = -4 * i32::try_from(body.len() + 1).expect("a short body");
    program.push(i().addi(X5, X5, -1).build());
    program.push(i().bne(X5, X0, back).build());
    program.push(i().addi(DONE_REG, X0, DONE).build());
    program.push(i().jal(X0, 0).build());
    program
}

fn cycles_to_finish(config: &Config, program: &[u32]) -> u64 {
    let mut ctx = TestContext::with_fast_memory(config).load_program(PROGRAM_BASE, program);
    ctx.run_until(100_000, |ctx| ctx.get_reg(DONE_REG) == DONE as u64).expect("program finished")
}

/// Cycles each extra iteration of the loop around `body` takes.
fn cycles_per_iteration(config: &Config, padding: usize, body: &[u32]) -> f64 {
    let (short, long) = ITERATIONS;
    let short_cycles = cycles_to_finish(config, &counted_loop(short, padding, body));
    let long_cycles = cycles_to_finish(config, &counted_loop(long, padding, body));
    (long_cycles - short_cycles) as f64 / f64::from(long - short)
}

fn assert_near(measured: f64, expected: f64, what: &str) {
    assert!(
        (measured - expected).abs() < 0.05,
        "{what}: {measured:.3} cycles, expected {expected}"
    );
}

/// A fetch block a four-instruction loop fits.
const BLOCK_BYTES: usize = 16;

/// Cycles per iteration of a two-nop loop (four instructions, one block's
/// worth) whose first instruction is `offset` instructions into its block.
fn loop_cycles(fetch_block_bytes: Option<usize>, offset: usize) -> f64 {
    let mut config = core();
    config.pipeline.fetch_block_bytes = fetch_block_bytes;
    let nop = InstructionBuilder::new().nop().build();
    // The counter's set-up takes the block's first slot.
    cycles_per_iteration(&config, offset - 1, &[nop, nop])
}

#[test]
fn a_loop_inside_one_fetch_block_fetches_in_a_cycle() {
    assert_near(loop_cycles(Some(BLOCK_BYTES), 4), 1.0, "aligned loop");
}

#[test]
fn a_loop_straddling_two_fetch_blocks_takes_two_cycles() {
    assert_near(loop_cycles(Some(BLOCK_BYTES), 2), 2.0, "straddling loop");
}

#[test]
fn without_a_fetch_block_fetch_reads_across_the_line() {
    assert_near(loop_cycles(None, 2), 1.0, "whole-line fetch");
}

/// Taken jumps in each iteration of the jump loop, before its branch back.
const JUMPS: usize = 8;
/// The main BTB's bubbles in the bubble tests.
const MAIN_BUBBLES: u64 = 2;
/// The micro-BTB's bubbles in the bubble tests.
const MICRO_BUBBLES: u64 = 1;
/// A loop of `JUMPS` jumps and its branch back fits in a micro-BTB this big.
const ROOMY_MICRO: usize = 64;
/// ...and thrashes one this big.
const SMALL_MICRO: usize = 4;

/// The core with the BTB bubbles and micro-BTB under test.
fn with_btbs(main_bubbles: u64, micro_btb: Option<MicroBtbConfig>) -> Config {
    let mut config = core();
    config.pipeline.btb_taken_bubbles = main_bubbles;
    config.pipeline.micro_btb = micro_btb;
    config
}

/// A fully associative micro-BTB of `size` entries.
const fn micro(size: usize) -> MicroBtbConfig {
    MicroBtbConfig { size, ways: size, taken_bubbles: MICRO_BUBBLES }
}

/// `JUMPS` jumps, each over a nop to the next.
fn jumps() -> Vec<u32> {
    let i = InstructionBuilder::new;
    (0..JUMPS).flat_map(|_| [i().jal(X0, 8).build(), i().nop().build()]).collect()
}

/// Cycles each taken branch of the jump loop costs on `config`.
fn cycles_per_taken_branch(config: &Config) -> f64 {
    cycles_per_iteration(config, 0, &jumps()) / (JUMPS + 1) as f64
}

#[test]
fn a_taken_branch_takes_a_cycle_without_bubbles() {
    assert_near(cycles_per_taken_branch(&with_btbs(0, None)), 1.0, "no bubbles");
}

#[test]
fn a_taken_branch_the_main_btb_predicts_costs_its_bubbles() {
    let measured = cycles_per_taken_branch(&with_btbs(MAIN_BUBBLES, None));

    assert_near(measured, 1.0 + MAIN_BUBBLES as f64, "main BTB");
}

#[test]
fn a_taken_branch_the_micro_btb_holds_costs_its_bubbles() {
    let measured = cycles_per_taken_branch(&with_btbs(MAIN_BUBBLES, Some(micro(ROOMY_MICRO))));

    assert_near(measured, 1.0 + MICRO_BUBBLES as f64, "micro-BTB");
}

#[test]
fn branches_past_the_micro_btb_cost_the_main_btbs_bubbles() {
    let measured = cycles_per_taken_branch(&with_btbs(MAIN_BUBBLES, Some(micro(SMALL_MICRO))));

    assert_near(measured, 1.0 + MAIN_BUBBLES as f64, "micro-BTB thrashed");
}
