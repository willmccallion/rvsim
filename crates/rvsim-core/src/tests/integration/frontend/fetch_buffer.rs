//! Fetch groups inside the line the I-cache last returned come from the
//! fetch buffer, so a straight-line stream pays the I-cache latency once
//! per line rather than once per group; a FENCE.I invalidates the buffer
//! with the cache, so the refetch after it misses.

use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x4000;
const X29: u32 = 29;
const X31: u32 = 31;
const LINE_BYTES: usize = 64;
const LINES: usize = 8;
const INSTS_PER_LINE: usize = LINE_BYTES / 4;
const WIDTH: usize = 4;
const I_CACHE_LATENCY: u64 = 8;
const FENCE_I: u32 = 0x0000_100f;

fn straight_line_program() -> Vec<u32> {
    let mut program = vec![InstructionBuilder::new().nop().build(); LINES * INSTS_PER_LINE];
    let last = program.len() - 1;
    program[last - 1] = InstructionBuilder::new().addi(X31, 0, 1).build();
    program[last] = InstructionBuilder::new().jal(0, 0).build();
    program
}

/// `iterations` times, inside one line: a FENCE.I, then the marker count
/// in x31. Once the loop branch predicts taken, fetch stays in the line.
fn fence_i_loop(iterations: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(X29, 0, iterations).build(),
        FENCE_I,
        i().addi(X31, X31, 1).build(),
        i().bne(X31, X29, -8).build(),
        i().jal(0, 0).build(),
    ]
}

/// Cycles until `program` leaves `marker` in x31, and the L1I misses it took.
fn run_until(backend: BackendKind, program: &[u32], marker: u64) -> (u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = WIDTH;
    config.cache.l1_i.enabled = true;
    config.cache.l1_i.line_bytes = LINE_BYTES;
    config.cache.l1_i.latency = I_CACHE_LATENCY;
    let mut tc = TestContext::new_with_config(&config)
        .with_memory(RAM_SIZE, RAM_BASE)
        .load_program(RAM_BASE, program);

    let mut cycles = 0;
    while tc.get_reg(X31 as usize) != marker {
        tc.run(1);
        cycles += 1;
        assert!(cycles < 10_000, "program never reached its marker");
    }
    let misses = tc.sim.state.cores[0].units.l1_i_cache.stat_paths.misses;
    (cycles, tc.sim.state.stats.get(misses).unwrap_or(0.0) as u64)
}

fn assert_same_line_groups_skip_the_icache(backend: BackendKind) {
    let groups = (LINES * INSTS_PER_LINE) / WIDTH;
    let per_group_bound = groups as u64 * (I_CACHE_LATENCY + 1);

    let (cycles, _) = run_until(backend, &straight_line_program(), 1);

    assert!(
        cycles < per_group_bound,
        "{cycles} cycles: every group paid the I-cache latency (bound {per_group_bound})"
    );
}

#[test]
fn in_order_same_line_groups_skip_the_icache() {
    assert_same_line_groups_skip_the_icache(BackendKind::InOrder);
}

#[test]
fn out_of_order_same_line_groups_skip_the_icache() {
    assert_same_line_groups_skip_the_icache(BackendKind::OutOfOrder);
}

/// FENCE.I drops every I-cache line, and with them the fetch buffer's copy
/// of the line the FENCE.I sits in, so the instructions after it refetch
/// through a miss, as Rocket's `flush_icache` makes them. The first
/// iterations warm the loop branch's prediction; after that fetch stays
/// in the line, and each further iteration costs exactly one miss.
fn assert_a_fence_i_refetches_through_the_icache(backend: BackendKind) {
    let (short_cycles, short_misses) = run_until(backend, &fence_i_loop(4), 4);

    let (long_cycles, long_misses) = run_until(backend, &fence_i_loop(8), 8);

    assert_eq!(
        long_misses - short_misses,
        4,
        "{backend:?}: each FENCE.I refetches its line through the I-cache"
    );
    assert!(
        long_cycles - short_cycles >= 4 * I_CACHE_LATENCY,
        "{backend:?}: {long_cycles} cycles for 8 FENCE.Is, {short_cycles} for 4"
    );
}

#[test]
fn in_order_fence_i_refetches_through_the_icache() {
    assert_a_fence_i_refetches_through_the_icache(BackendKind::InOrder);
}

#[test]
fn out_of_order_fence_i_refetches_through_the_icache() {
    assert_a_fence_i_refetches_through_the_icache(BackendKind::OutOfOrder);
}
