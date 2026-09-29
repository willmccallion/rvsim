//! Commit counts every branch's and jump's prediction, as gem5's
//! `branchPred.mispredicted` counts every committed control instruction.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::BackendKind;
use rvsim_core::config::{BranchPredictorKind, Config};

const PROGRAM_BASE: u64 = 0x8000_0000;
const ITERATIONS: i32 = 20;
const DONE_REG: usize = 31;
const DONE: u64 = 7;

/// A loop whose `jalr` alternates between two targets, so a BTB that
/// remembers the last one is wrong every iteration.
fn alternating_indirect_jump() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let nop = i().nop().build();
    vec![
        i().addi(5, 0, ITERATIONS).build(), // 0x00
        i().auipc(6, 0).build(),            // 0x04
        i().addi(6, 6, 0x1c).build(),       // 0x08: x6 = 0x20
        i().addi(7, 0, 8).build(),          // 0x0c
        i().jalr(0, 6, 0).build(),          // 0x10: to 0x20 or 0x28
        nop,                                // 0x14
        nop,                                // 0x18
        nop,                                // 0x1c
        nop,                                // 0x20
        i().jal(0, 8).build(),              // 0x24: to 0x2c
        nop,                                // 0x28
        i().xor(6, 6, 7).build(),           // 0x2c: swap targets
        i().addi(5, 5, -1).build(),         // 0x30
        i().bne(5, 0, -0x24).build(),       // 0x34: to 0x10
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]
}

fn committed_mispredicts(backend: BackendKind) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.branch_predictor = BranchPredictorKind::GShare;
    let mut ctx = TestContext::new_with_config(&config)
        .load_program(PROGRAM_BASE, &alternating_indirect_jump());
    ctx.run_until(20_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished");
    let path = ctx.sim.state.cores[0].units.stat_paths.bp.committed_mispredicts;
    ctx.sim.state.stats.get(path).unwrap_or(0.0) as u64
}

#[test]
fn a_mispredicted_jump_counts_as_a_committed_misprediction() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mispredicts = committed_mispredicts(backend);

        assert!(
            mispredicts >= ITERATIONS as u64 - 2,
            "{backend:?}: {mispredicts} committed mispredictions"
        );
    }
}
