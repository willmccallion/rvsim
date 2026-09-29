//! The out-of-order backend writes back at most `writeback_width` results
//! per cycle (gem5's `wbWidth`); results beyond that wait for later slots.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::uarch::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const ADDS_PER_ITERATION: usize = 32;
const ITERATIONS: usize = 50;
const COUNTER: u32 = 28;
const DONE_REG: usize = 31;
const DONE: u64 = 7;

/// A loop of independent adds, long enough for the warm front end to keep
/// the backend fed.
fn independent_adds() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().addi(COUNTER, 0, ITERATIONS as i32).build()];
    program
        .extend((0..ADDS_PER_ITERATION).map(|n| i().addi(5 + (n % 8) as u32, 0, n as i32).build()));
    let body_bytes = 4 * (ADDS_PER_ITERATION as i32 + 1);
    program.push(i().addi(COUNTER, COUNTER, -1).build());
    program.push(i().bne(COUNTER, 0, -body_bytes).build());
    program.push(i().addi(DONE_REG as u32, 0, DONE as i32).build());
    program.push(i().jal(0, 0).build());
    program
}

fn cycles_with_writeback_width(writeback_width: usize) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = BackendType::OutOfOrder;
    config.pipeline.width = 4;
    config.pipeline.writeback_width = Some(writeback_width);
    config.cache.l1_i.enabled = true;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &independent_adds());
    ctx.run_until(50_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished")
}

#[test]
fn a_single_writeback_port_writes_back_one_result_per_cycle() {
    let results = (ADDS_PER_ITERATION + 2) * ITERATIONS;

    let narrow = cycles_with_writeback_width(1);
    let wide = cycles_with_writeback_width(4);

    assert!(narrow >= results as u64, "narrow {narrow}: one result per cycle");
    assert!(wide * 2 < narrow, "wide {wide} vs narrow {narrow}");
}
