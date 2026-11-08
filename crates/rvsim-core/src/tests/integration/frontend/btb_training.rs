//! The BTB learns a target only when a misprediction is corrected, as in
//! gem5.
//!
//! An indirect jump executed down a path an older branch squashes before
//! the jump's own correction arrives leaves no trace in it.

use crate::common::{InstSeq, VirtAddr};
use crate::config::BackendKind;
use crate::config::{BranchPredictorKind, Config};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::regs::{T0, T1, T2, T3, X0};
use crate::uarch::bpred::ControlInst;
const PROGRAM_BASE: u64 = 0x8000_0000;
/// The indirect jump only the wrong path reaches.
const WRONG_PATH_JALR: u64 = PROGRAM_BASE + 0x14;

/// A divide delays a taken branch the static predictor calls not taken.
/// The wrong path's `jalr` takes its target from the divide too, so it
/// executes right after the branch resolves, and the branch's squash
/// arrives before the `jalr`'s own.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, X0, 7).build(),
        i().div(T1, T0, T0).build(),
        i().bne(T1, X0, 20).build(),
        i().addi(T3, X0, 0xff).build(),
        i().add(T2, T1, T3).build(),
        i().jalr(X0, T2, 0).build(),
        i().addi(X0, X0, 0).build(),
        i().jal(X0, 0).build(),
    ]
}

#[test]
fn a_wrong_path_indirect_jump_leaves_the_btb_alone() {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.branch_predictor = BranchPredictorKind::Static;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());

    ctx.run(200);

    let jump = ControlInst::IndirectJump { returns: false, link: None };
    let predictor = &mut ctx.sim.state.cores[0].units.branch_predictor;
    assert_eq!(
        predictor.predict(InstSeq::new(u64::MAX), VirtAddr::new(WRONG_PATH_JALR), jump),
        None
    );
}
