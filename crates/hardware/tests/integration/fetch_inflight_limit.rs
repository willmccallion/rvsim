//! Fetch1 keeps at most one fetch group in flight.
//!
//! It waits for the I-cache response before issuing the next group, as
//! gem5's fetch stage does (`IcacheWaitResponse`), and waits for fetch2 to
//! drain the fetch1→fetch2 latch. Without both gates fetch runs arbitrarily
//! far ahead of the backend and the in-flight tables grow without bound.
//! A group is one line-sized request, so at most one request is pending.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::uarch::pipeline::engine::{BackendType, ExecutionEngine, PipelineDispatch};

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x4000;
const PROGRAM_LEN: usize = 512;

fn inflight_fetches(pipeline: &PipelineDispatch) -> usize {
    match pipeline {
        PipelineDispatch::InOrder(p) => {
            p.engine.common().outstanding_fetches.len() + p.engine.common().fetch_reorder.len()
        }
        PipelineDispatch::OutOfOrder(p) => {
            p.engine.common().outstanding_fetches.len() + p.engine.common().fetch_reorder.len()
        }
    }
}

fn straight_line_program() -> Vec<u32> {
    let mut program = vec![InstructionBuilder::new().nop().build(); PROGRAM_LEN];
    let last = program.len() - 1;
    program[last] = InstructionBuilder::new().jal(0, 0).build();
    program
}

fn assert_fetch_bounded(backend: BackendType) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    let width = config.pipeline.width;
    let mut tc = TestContext::new_with_config(&config)
        .with_memory(RAM_SIZE, RAM_BASE)
        .load_program(RAM_BASE, &straight_line_program());

    for cycle in 0..2_000 {
        tc.run(1);
        let inflight = inflight_fetches(&tc.sim.state.cores[0].pipeline);
        let latched = tc.sim.state.cores[0].pipeline.snapshot(width).fetch1_fetch2.len();
        assert!(
            inflight <= 1,
            "cycle {cycle}: {inflight} fetch groups in flight, expected at most one"
        );
        assert!(
            latched <= width,
            "cycle {cycle}: fetch1->fetch2 latch holds {latched} entries, more than width {width}"
        );
    }
}

#[test]
fn in_order_fetch_never_exceeds_one_group_in_flight() {
    assert_fetch_bounded(BackendType::InOrder);
}

#[test]
fn out_of_order_fetch_never_exceeds_one_group_in_flight() {
    assert_fetch_bounded(BackendType::OutOfOrder);
}
