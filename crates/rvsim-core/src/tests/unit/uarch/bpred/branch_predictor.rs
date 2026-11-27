//! The branch predictor the pipeline holds hands each event to the unit of
//! the configured kind.

use crate::common::{InstSeq, VirtAddr};
use crate::config::Config;
use crate::uarch::bpred::{BranchPredictor, ControlInst};

const PC: VirtAddr = VirtAddr::new(0x1000);
const TARGET: VirtAddr = VirtAddr::new(0x2000);
const REAL_TARGET: VirtAddr = VirtAddr::new(0x3000);
const JUMP: ControlInst = ControlInst::Jump { target: TARGET, link: None };

fn predictor() -> BranchPredictor {
    BranchPredictor::new(&Config::minimal())
}

#[test]
fn squashing_after_a_prediction_drops_only_the_younger_ones() {
    let mut bp = predictor();
    let (older, younger) = (InstSeq::new(1), InstSeq::new(2));
    let _ = bp.predict(older, PC, JUMP);
    let _ = bp.predict(younger, VirtAddr::new(PC.val() + 8), JUMP);

    bp.squash_after(older);

    assert!(bp.is_predicted(older));
    assert!(!bp.is_predicted(younger));
}

/// Decode found that the instruction fetch predicted as a jump from the
/// BTB is not a control instruction.
#[test]
fn forgetting_a_prediction_drops_it_and_its_btb_entry() {
    let mut bp = predictor();
    let seq = InstSeq::new(1);
    let _ = bp.discover(seq, PC, JUMP);

    bp.forget(seq, PC);

    assert!(!bp.is_predicted(seq));
    assert!(bp.btb_lookup(PC).is_none());
}

#[test]
fn correcting_a_stale_target_teaches_the_btb_the_real_one() {
    let mut bp = predictor();
    let seq = InstSeq::new(1);
    let _ = bp.discover(seq, PC, JUMP);

    bp.correct_target(seq, REAL_TARGET);

    assert_eq!(bp.btb_lookup(PC).map(|prediction| prediction.hit.target), Some(REAL_TARGET));
}
