//! An unconditional jump shifts the global branch history as taken, the
//! way gem5's predictors treat every control instruction, so a branch
//! after it sees a different path from one reached with no jump.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;
use rvsim_core::core::units::bru::BranchPredictorWrapper;

const PROGRAM_BASE: u64 = 0x8000_0000;

fn history_after_a_jump_loop(backend: BackendType) -> bool {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.branch_predictor = rvsim_core::config::BranchPredictor::GShare;
    config.system.console = rvsim_core::config::Console::Quiet;
    let program = [InstructionBuilder::new().jal(0, 0).build()];
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);

    ctx.run(100);

    assert!(ctx.sim.state.harts[0].instructions_retired > 4, "{backend:?}: the loop ran");
    let BranchPredictorWrapper::GShare(unit) = &ctx.sim.state.cores[0].units.branch_predictor
    else {
        panic!("{backend:?}: configured for GShare");
    };
    unit.direction().history() & 1 == 1
}

#[test]
fn a_jump_enters_the_global_history_as_taken_o3() {
    assert!(history_after_a_jump_loop(BackendType::OutOfOrder));
}

#[test]
fn a_jump_enters_the_global_history_as_taken_inorder() {
    assert!(history_after_a_jump_loop(BackendType::InOrder));
}
