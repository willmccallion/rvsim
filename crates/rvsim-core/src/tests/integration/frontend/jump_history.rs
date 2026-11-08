//! An unconditional jump shifts the global branch history as taken, the
//! way gem5's predictors treat every control instruction, so a branch
//! after it sees a different path from one reached with no jump.

use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::X0;
use crate::uarch::bpred::BranchPredictor;

fn history_after_a_jump_loop(backend: BackendKind) -> bool {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.branch_predictor = crate::config::BranchPredictorKind::GShare;
    config.system.console = crate::config::Console::Quiet;
    let program = [InstructionBuilder::new().jal(X0, 0).build()];
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program);

    ctx.run(100);

    assert!(ctx.sim.state.harts[0].instructions_retired > 4, "{backend:?}: the loop ran");
    let BranchPredictor::GShare(unit) = &ctx.sim.state.cores[0].units.branch_predictor else {
        panic!("{backend:?}: configured for GShare");
    };
    unit.direction().history() & 1 == 1
}

#[test]
fn a_jump_enters_the_global_history_as_taken_o3() {
    assert!(history_after_a_jump_loop(BackendKind::OutOfOrder));
}

#[test]
fn a_jump_enters_the_global_history_as_taken_inorder() {
    assert!(history_after_a_jump_loop(BackendKind::InOrder));
}
