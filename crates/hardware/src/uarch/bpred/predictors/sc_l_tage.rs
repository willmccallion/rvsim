//! SC-L-TAGE + ITTAGE Composed Branch Predictor.
//!
//! Combines TAGE (direction), Loop Predictor (counted loop override),
//! Statistical Corrector (correction layer), and ITTAGE (indirect targets)
//! into a single high-accuracy predictor. This is the "best possible"
//! composed predictor following Seznec's CBP-winning designs.
//!
//! Prediction flow:
//! 1. TAGE base -> (direction, `TageScMeta`)
//! 2. Loop predictor override -> if confident, use loop prediction
//! 3. SC correction -> may flip direction if confident base is wrong
//! 4. Target: ITTAGE for indirect jumps, the unit's BTB otherwise

use crate::config::{IttageConfig, LoopConfig, ScConfig, TageConfig};
use crate::uarch::bpred::Ghr;
use crate::uarch::bpred::components::{
    ittage::Ittage,
    loop_predictor::{LoopPrediction, LoopPredictor},
    stat_corrector::{ScPrediction, StatCorrector},
    tage_core::{TageCore, TagePrediction},
    tage_history::{HistoryBranch, HistoryCheckpoint},
};
use crate::uarch::bpred::direction::{DirectionPredictor, Jump, Retired};

/// SC-L-TAGE + ITTAGE composed predictor.
#[derive(Debug)]
pub struct ScLTagePredictor {
    /// ITTAGE's speculative global history.
    spec_ghr: Ghr,
    /// ITTAGE's committed global history.
    commit_ghr: Ghr,

    /// Shared TAGE direction core.
    tage: TageCore,

    loop_pred: LoopPredictor,
    sc: StatCorrector,
    ittage: Ittage,
}

/// What an SC-L-TAGE prediction was made with.
#[derive(Clone, Copy, Debug)]
pub struct ScLTageHistory {
    /// ITTAGE's speculative global history before the prediction.
    ghr: Ghr,
    /// TAGE's histories before the prediction.
    checkpoint: HistoryCheckpoint,
    branch: HistoryBranch,
    /// What a conditional branch's prediction read.
    conditional: Option<ConditionalPrediction>,
}

/// The entries each direction component read for a conditional branch.
#[derive(Clone, Copy, Debug)]
struct ConditionalPrediction {
    tage: TagePrediction,
    loop_prediction: LoopPrediction,
    sc: ScPrediction,
}

impl ScLTagePredictor {
    /// Creates a new SC-L-TAGE + ITTAGE predictor.
    pub fn new(
        tage_config: &TageConfig,
        sc_config: &ScConfig,
        ittage_config: &IttageConfig,
        loop_config: &LoopConfig,
    ) -> Self {
        let ittage_history = ittage_config.history_lengths.iter().copied().max().unwrap_or(0);
        Self {
            spec_ghr: Ghr::with_len(ittage_history),
            commit_ghr: Ghr::with_len(ittage_history),
            tage: TageCore::new(tage_config),
            loop_pred: LoopPredictor::new(loop_config),
            sc: StatCorrector::new(sc_config),
            ittage: Ittage::new(ittage_config),
        }
    }

    fn push_speculative(&mut self, pc: u64, taken: bool, branch: HistoryBranch) {
        self.tage.speculate(pc, taken, branch);
        self.ittage.speculate(taken, &self.spec_ghr);
        self.spec_ghr.push(taken);
    }

    fn record(&self, branch: HistoryBranch) -> ScLTageHistory {
        ScLTageHistory {
            ghr: self.spec_ghr,
            checkpoint: self.tage.checkpoint(),
            branch,
            conditional: None,
        }
    }

    /// Advances the committed history. Must follow every read of the
    /// committed CSRs for this instruction: the folded histories need the
    /// bit about to be shifted out.
    fn push_committed(&mut self, taken: bool) {
        self.ittage.commit_advance(taken, &self.commit_ghr);
        self.commit_ghr.push(taken);
    }

    fn speculate_conditional(&mut self, prediction: &ConditionalPrediction, taken: bool) {
        self.loop_pred.speculate(&prediction.loop_prediction, taken);
        self.sc.speculate(&prediction.sc, taken);
    }

    fn squash_conditional(&mut self, prediction: &ConditionalPrediction) {
        self.loop_pred.squash(&prediction.loop_prediction);
        self.sc.squash(&prediction.sc);
    }

    fn train_direction(&mut self, pc: u64, taken: bool, prediction: &ConditionalPrediction) {
        self.sc.update(&prediction.sc, taken);
        self.loop_pred.commit(pc, taken, prediction.tage.taken(), prediction.sc.taken());
        self.tage.update(taken, &prediction.tage, prediction.sc.taken());
    }
}

impl DirectionPredictor for ScLTagePredictor {
    type History = ScLTageHistory;

    fn lookup(&self, pc: u64, target: u64) -> (bool, ScLTageHistory) {
        let tage = self.tage.predict(pc);
        let loop_prediction = self.loop_pred.predict(pc);
        let before_sc = match loop_prediction.confident() {
            Some(loop_taken) if self.loop_pred.in_use() => loop_taken,
            _ => tage.taken(),
        };
        let path = u64::from(self.tage.path_history());
        let sc = self.sc.predict(pc, target, path, tage.meta(), before_sc);
        let mut history = self.record(HistoryBranch::Conditional);
        history.conditional = Some(ConditionalPrediction { tage, loop_prediction, sc });
        (sc.taken(), history)
    }

    fn unconditional(&self, _pc: u64, jump: Jump) -> ScLTageHistory {
        self.record(HistoryBranch::from(jump))
    }

    fn update_histories(&mut self, pc: u64, taken: bool, history: &ScLTageHistory) {
        self.push_speculative(pc, taken, history.branch);
        if let Some(prediction) = &history.conditional {
            self.speculate_conditional(prediction, taken);
        }
    }

    fn squash(&mut self, history: &ScLTageHistory) {
        self.spec_ghr = history.ghr;
        self.tage.restore(&history.checkpoint);
        if let Some(prediction) = &history.conditional {
            self.squash_conditional(prediction);
        }
    }

    fn squash_done(&mut self) {
        self.ittage.repair_history(&self.spec_ghr);
    }

    fn correct(&mut self, pc: u64, taken: bool, history: &ScLTageHistory) {
        self.spec_ghr = history.ghr;
        self.tage.restore(&history.checkpoint);
        self.ittage.repair_history(&self.spec_ghr);
        self.push_speculative(pc, taken, history.branch);
        if let Some(prediction) = &history.conditional {
            self.squash_conditional(prediction);
            self.speculate_conditional(prediction, taken);
        }
    }

    fn commit(&mut self, pc: u64, retired: Retired, history: &ScLTageHistory) {
        if let Some(prediction) = &history.conditional {
            self.train_direction(pc, retired.taken, prediction);
        }
        if let Some(target) = retired.indirect_target {
            self.ittage.update(pc, target, &history.ghr);
        }
        self.push_committed(retired.taken);
    }

    fn indirect_target(&self, pc: u64) -> Option<u64> {
        self.ittage.predict(pc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uarch::bpred::direction::BranchClass;

    fn test_tage_config() -> TageConfig {
        TageConfig {
            num_banks: 4,
            table_size: 256,
            reset_interval: 100_000,
            history_lengths: vec![5, 15, 44, 130],
            tag_widths: vec![9, 9, 10, 10],
            ..TageConfig::default()
        }
    }

    fn test_ittage_config() -> IttageConfig {
        IttageConfig {
            num_banks: 4,
            table_size: 64,
            history_lengths: vec![4, 8, 16, 32],
            tag_widths: vec![9, 9, 10, 10],
            reset_interval: 100_000,
        }
    }

    fn predictor() -> ScLTagePredictor {
        ScLTagePredictor::new(
            &test_tage_config(),
            &ScConfig::default(),
            &test_ittage_config(),
            &LoopConfig::default(),
        )
    }

    #[test]
    fn an_untrained_branch_follows_the_weakly_not_taken_bimodal() {
        let (taken, _) = predictor().lookup(0x8000_1004, 0x8000_1040);

        assert!(!taken);
    }

    #[test]
    fn squashing_younger_predictions_restores_the_history_they_shifted() {
        let mut pred = predictor();
        for i in 0u64..20 {
            let history = pred.unconditional(0x1000 + i * 4, Jump::Direct);
            pred.update_histories(0x1000 + i * 4, i % 2 == 0, &history);
        }
        let before = (pred.spec_ghr, pred.tage.checkpoint());
        let mut squashed = Vec::new();
        for _ in 0..10 {
            let (_, history) = pred.lookup(0x2000, 0x2040);
            pred.update_histories(0x2000, true, &history);
            squashed.push(history);
        }

        for history in squashed.iter().rev() {
            pred.squash(history);
        }
        pred.squash_done();

        assert_eq!((pred.spec_ghr, pred.tage.checkpoint()), before);
    }

    #[test]
    fn a_committed_indirect_jump_teaches_ittage_its_target() {
        let mut pred = predictor();
        let pc = 0x8000_2000u64;
        let target = 0x8000_5000u64;
        let history = pred.unconditional(pc, Jump::Indirect);

        pred.commit(
            pc,
            Retired {
                class: BranchClass::Unconditional,
                taken: true,
                indirect_target: Some(target),
            },
            &history,
        );

        assert_eq!(pred.indirect_target(pc), Some(target));
    }
}
