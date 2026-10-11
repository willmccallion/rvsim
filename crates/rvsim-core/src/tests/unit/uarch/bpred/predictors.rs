//! Branch predictor direction tests.
//!
//! Each predictor sits in a `BranchPredUnit` and is driven the way the
//! pipeline drives it: a prediction, a correction when it was wrong, and a
//! commit.

use crate::common::InstSeq;
use crate::config::{BhtConfig, Config, PerceptronConfig, TageConfig, TournamentConfig};
use crate::uarch::bpred::predictors::bht::BhtPredictor;
use crate::uarch::bpred::predictors::gshare::GSharePredictor;
use crate::uarch::bpred::predictors::perceptron::PerceptronPredictor;
use crate::uarch::bpred::predictors::sc_l_tage::ScLTagePredictor;
use crate::uarch::bpred::predictors::static_bp::StaticPredictor;
use crate::uarch::bpred::predictors::tage::TagePredictor;
use crate::uarch::bpred::predictors::tournament::TournamentPredictor;
use crate::uarch::bpred::{BranchPredUnit, ControlInst, DirectionPredictor};

const PC: u64 = 0x1000;
const TARGET: u64 = 0x2000;

/// A prediction unit and the next sequence number fetch would give out.
struct Driver<P: DirectionPredictor> {
    unit: BranchPredUnit<P>,
    next_seq: u64,
}

impl<P: DirectionPredictor> Driver<P> {
    fn new(direction: P) -> Self {
        Self { unit: BranchPredUnit::new(direction, 64, 4, 8), next_seq: 0 }
    }

    fn predict(&mut self, pc: u64, inst: ControlInst) -> (InstSeq, Option<u64>) {
        let seq = InstSeq::new(self.next_seq);
        self.next_seq += 1;
        (seq, self.unit.predict(seq, pc, inst))
    }

    /// Predicts, corrects and commits one conditional branch; returns
    /// whether it was predicted taken.
    fn run_branch(&mut self, pc: u64, taken: bool) -> bool {
        let (seq, target) = self.predict(pc, ControlInst::Branch { target: TARGET });
        let predicted = target.is_some();
        if predicted != taken {
            self.unit.mispredict(seq, taken, TARGET);
        }
        self.unit.commit(seq);
        predicted
    }

    fn train(&mut self, pc: u64, taken: bool, n: usize) {
        for _ in 0..n {
            let _ = self.run_branch(pc, taken);
        }
    }

    /// The direction the next prediction of `pc` takes, then squashed.
    fn predicts_taken(&mut self, pc: u64) -> bool {
        let (_, target) = self.predict(pc, ControlInst::Branch { target: TARGET });
        self.unit.squash_all();
        target.is_some()
    }
}

fn tage() -> TagePredictor {
    TagePredictor::new(&TageConfig {
        num_banks: 4,
        table_size: 2048,
        reset_interval: 256_000,
        history_lengths: vec![5, 15, 44, 130],
        tag_widths: vec![9, 9, 10, 10],
        ..TageConfig::default()
    })
}

fn perceptron() -> PerceptronPredictor {
    PerceptronPredictor::new(&PerceptronConfig { history_length: 8, table_bits: 6 })
}

fn tournament() -> TournamentPredictor {
    TournamentPredictor::new(&TournamentConfig {
        global_size_bits: 6,
        local_hist_bits: 6,
        local_pred_bits: 6,
    })
}

#[test]
fn static_predicts_not_taken() {
    assert!(!Driver::new(StaticPredictor::new()).predicts_taken(PC));
}

#[test]
fn static_ignores_training() {
    let mut bp = Driver::new(StaticPredictor::new());
    bp.train(PC, true, 100);
    assert!(!bp.predicts_taken(PC));
}

#[test]
fn gshare_starts_weakly_not_taken() {
    assert!(!Driver::new(GSharePredictor::new()).predicts_taken(PC));
}

#[test]
fn gshare_learns_taken() {
    let mut bp = Driver::new(GSharePredictor::new());
    bp.train(PC, true, 20);
    assert!(bp.predicts_taken(PC));
}

#[test]
fn gshare_learns_not_taken_after_taken() {
    let mut bp = Driver::new(GSharePredictor::new());
    bp.train(PC, true, 10);
    bp.train(PC, false, 20);
    assert!(!bp.predicts_taken(PC));
}

fn bht() -> BhtPredictor {
    BhtPredictor::new(&BhtConfig::default())
}

#[test]
fn bht_starts_not_taken() {
    assert!(!Driver::new(bht()).predicts_taken(PC));
}

/// Each outcome shifts the history the next lookup indexes with: after
/// eight taken outcomes the history is all ones, and the ninth trains the
/// counter the next lookup reads.
#[test]
fn bht_learns_taken_under_a_history_it_has_been_trained_under() {
    let mut bp = Driver::new(bht());

    bp.train(PC, true, 9);

    assert!(bp.predicts_taken(PC));
}

/// `BHT.index`: the PC's word address in the low index bits, the history
/// multiplied by floor(sqrt(3) / 2 * 2^8) = 221 and the top 3 bits of the
/// product's low byte placed above them.
#[test]
fn bht_indexes_by_the_word_address_with_the_hashed_history_above_it() {
    let bp = bht();
    let pc = 0x8000_0010;

    assert_eq!(bp.index(pc, 0), 4);
    // 221 * 160 = 35360, whose low byte 32 has 1 in its top three bits.
    assert_eq!(bp.index(pc, 160), 4 ^ (1 << 6));
}

/// Two branches a few instructions apart never share a counter, whatever
/// their histories, which a PC-XOR-history index does not guarantee.
#[test]
fn bht_keeps_nearby_branches_apart_under_every_history() {
    let bp = bht();
    for history in 0..256 {
        for other in 0..256 {
            assert_ne!(bp.index(0x8000_0010, history), bp.index(0x8000_0018, other));
        }
    }
}

/// The history keeps its newest outcome in its top bit, and a
/// misprediction rebuilds it from the branch's fetch-time history.
#[test]
fn bht_history_shifts_the_newest_outcome_in_at_the_top() {
    let mut bp = Driver::new(bht());

    let _ = bp.run_branch(PC, true);
    assert_eq!(bp.unit.direction().history(), 1 << 7);
    let _ = bp.run_branch(PC, false);
    assert_eq!(bp.unit.direction().history(), 1 << 6);
}

#[test]
fn bht_resets_the_history_to_a_mispredicted_jumps_fetch_time_history() {
    let mut bp = Driver::new(bht());
    bp.train(PC, true, 2);
    let before = bp.unit.direction().history();

    let (seq, _) = bp.predict(PC + 8, ControlInst::IndirectJump { returns: false, link: None });
    bp.unit.mispredict(seq, true, TARGET);

    assert_eq!(bp.unit.direction().history(), before);
}

#[test]
fn perceptron_starts_taken_with_zero_weights() {
    assert!(Driver::new(perceptron()).predicts_taken(PC));
}

#[test]
fn perceptron_learns_not_taken() {
    let mut bp = Driver::new(perceptron());
    bp.train(PC, false, 100);
    assert!(!bp.predicts_taken(PC));
}

#[test]
fn perceptron_retrains() {
    let mut bp = Driver::new(perceptron());
    bp.train(PC, true, 50);
    let first = bp.predicts_taken(PC);
    bp.train(PC, false, 100);

    assert!(first);
    assert!(!bp.predicts_taken(PC));
}

#[test]
fn tage_starts_taken_from_its_base_predictor() {
    assert!(Driver::new(tage()).predicts_taken(PC));
}

#[test]
fn tage_learns_not_taken() {
    let mut bp = Driver::new(tage());
    bp.train(PC, false, 40);
    assert!(!bp.predicts_taken(PC));
}

#[test]
fn tage_adapts_to_a_pattern_change() {
    let mut bp = Driver::new(tage());
    bp.train(PC, false, 30);
    let first = bp.predicts_taken(PC);
    bp.train(PC, true, 60);

    assert!(!first);
    assert!(bp.predicts_taken(PC));
}

#[test]
fn tournament_starts_not_taken_with_zeroed_counters() {
    assert!(!Driver::new(tournament()).predicts_taken(PC));
}

#[test]
fn tournament_learns_taken() {
    let mut bp = Driver::new(tournament());
    bp.train(PC, true, 20);
    assert!(bp.predicts_taken(PC));
}

#[test]
fn tournament_learns_not_taken_after_taken() {
    let mut bp = Driver::new(tournament());
    bp.train(PC, true, 10);
    bp.train(PC, false, 30);
    assert!(!bp.predicts_taken(PC));
}

#[test]
fn tournament_learns_an_alternating_branch() {
    let mut bp = Driver::new(tournament());
    let mut late_mispredictions = 0;
    for i in 0..200 {
        let taken = i % 2 == 0;
        if bp.run_branch(PC, taken) != taken && i >= 100 {
            late_mispredictions += 1;
        }
    }
    assert_eq!(late_mispredictions, 0);
}

#[test]
fn sc_l_tage_learns_an_indirect_jump_target_from_its_commits() {
    let pipeline = Config::minimal().pipeline;
    let mut bp = Driver::new(ScLTagePredictor::new(
        &pipeline.tage,
        &pipeline.sc,
        &pipeline.ittage,
        &pipeline.loop_predictor,
    ));
    let jump = ControlInst::IndirectJump { returns: false, link: None };
    let mut last_prediction = None;
    for _ in 0..20 {
        let (seq, predicted) = bp.predict(PC, jump);
        if predicted != Some(TARGET) {
            bp.unit.mispredict(seq, true, TARGET);
        }
        bp.unit.commit(seq);
        last_prediction = predicted;
    }

    assert_eq!(last_prediction, Some(TARGET));
}

/// An indirect jump that mispredicts teaches the BTB its real target.
fn a_mispredicted_indirect_jump_trains_the_btb<P: DirectionPredictor>(direction: P) {
    let mut bp = Driver::new(direction);
    let jump = ControlInst::IndirectJump { returns: false, link: None };
    let (seq, predicted) = bp.predict(PC, jump);
    assert_eq!(predicted, None);
    bp.unit.mispredict(seq, true, TARGET);
    bp.unit.commit(seq);

    assert_eq!(bp.predict(PC, jump).1, Some(TARGET));
}

#[test]
fn a_predictor_without_an_indirect_predictor_keeps_indirect_targets_in_the_btb() {
    a_mispredicted_indirect_jump_trains_the_btb(StaticPredictor::new());
    a_mispredicted_indirect_jump_trains_the_btb(GSharePredictor::new());
    a_mispredicted_indirect_jump_trains_the_btb(perceptron());
    a_mispredicted_indirect_jump_trains_the_btb(tage());
    a_mispredicted_indirect_jump_trains_the_btb(tournament());
}

#[test]
fn a_correctly_predicted_indirect_jump_leaves_the_btb_alone() {
    let mut bp = Driver::new(StaticPredictor::new());
    let jump = ControlInst::IndirectJump { returns: false, link: None };
    let (first, _) = bp.predict(PC, jump);
    bp.unit.mispredict(first, true, TARGET);
    bp.unit.commit(first);
    let (second, predicted) = bp.predict(PC, jump);
    assert_eq!(predicted, Some(TARGET));
    bp.unit.commit(second);

    assert_eq!(bp.predict(PC, jump).1, Some(TARGET));
}

#[test]
fn a_return_predicts_the_address_its_call_pushed() {
    let mut bp = Driver::new(StaticPredictor::new());
    let _ = bp.predict(PC, ControlInst::Jump { target: TARGET, link: Some(PC + 4) });

    let (_, target) = bp.predict(TARGET, ControlInst::IndirectJump { returns: true, link: None });

    assert_eq!(target, Some(PC + 4));
}

#[test]
fn squashing_a_return_and_a_call_restores_the_entry_the_call_overwrote() {
    let mut bp = Driver::new(StaticPredictor::new());
    let _ = bp.predict(0x100, ControlInst::Jump { target: TARGET, link: Some(0x1000) });
    let (kept, _) = bp.predict(0x200, ControlInst::Jump { target: TARGET, link: Some(0x2000) });
    let ret = ControlInst::IndirectJump { returns: true, link: None };
    let _ = bp.predict(0x300, ret);
    let _ = bp.predict(0x400, ControlInst::Jump { target: TARGET, link: Some(0x3000) });

    bp.unit.squash_after(kept);

    assert_eq!(bp.predict(0x500, ret).1, Some(0x2000));
    assert_eq!(bp.predict(0x600, ret).1, Some(0x1000));
}

#[test]
fn squashing_every_prediction_restores_the_global_history() {
    let mut bp = Driver::new(GSharePredictor::new());
    bp.train(PC, true, 3);
    let committed = bp.unit.direction().history();
    for _ in 0..5 {
        let _ = bp.predict(PC, ControlInst::Jump { target: TARGET, link: None });
    }

    bp.unit.squash_all();

    assert_eq!(bp.unit.direction().history(), committed);
}
