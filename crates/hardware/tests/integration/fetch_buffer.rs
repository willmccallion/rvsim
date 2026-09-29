//! Fetch groups inside the line the I-cache last returned come from the
//! fetch buffer, so a straight-line stream pays the I-cache latency once
//! per line rather than once per group.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::uarch::pipeline::engine::BackendType;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x4000;
const X31: u32 = 31;
const LINE_BYTES: usize = 64;
const LINES: usize = 8;
const INSTS_PER_LINE: usize = LINE_BYTES / 4;
const WIDTH: usize = 4;
const I_CACHE_LATENCY: u64 = 8;

fn straight_line_program() -> Vec<u32> {
    let mut program = vec![InstructionBuilder::new().nop().build(); LINES * INSTS_PER_LINE];
    let last = program.len() - 1;
    program[last - 1] = InstructionBuilder::new().addi(X31, 0, 1).build();
    program[last] = InstructionBuilder::new().jal(0, 0).build();
    program
}

fn cycles_to_marker(backend: BackendType) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = WIDTH;
    config.cache.l1_i.enabled = true;
    config.cache.l1_i.line_bytes = LINE_BYTES;
    config.cache.l1_i.latency = I_CACHE_LATENCY;
    let mut tc = TestContext::new_with_config(&config)
        .with_memory(RAM_SIZE, RAM_BASE)
        .load_program(RAM_BASE, &straight_line_program());

    let mut cycles = 0;
    while tc.get_reg(X31 as usize) != 1 {
        tc.run(1);
        cycles += 1;
        assert!(cycles < 10_000, "program never reached its marker");
    }
    cycles
}

fn assert_same_line_groups_skip_the_icache(backend: BackendType) {
    let groups = (LINES * INSTS_PER_LINE) / WIDTH;
    let per_group_bound = groups as u64 * (I_CACHE_LATENCY + 1);

    let cycles = cycles_to_marker(backend);

    assert!(
        cycles < per_group_bound,
        "{cycles} cycles: every group paid the I-cache latency (bound {per_group_bound})"
    );
}

#[test]
fn in_order_same_line_groups_skip_the_icache() {
    assert_same_line_groups_skip_the_icache(BackendType::InOrder);
}

#[test]
fn out_of_order_same_line_groups_skip_the_icache() {
    assert_same_line_groups_skip_the_icache(BackendType::OutOfOrder);
}
