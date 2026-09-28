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

use crate::config::{IttageConfig, ScConfig, TageConfig};
use crate::core::units::bru::Ghr;
use crate::core::units::bru::components::{
    ittage::Ittage,
    loop_predictor::LoopPredictor,
    sc_types::ScSum,
    sc_types::TageScMeta,
    stat_corrector::StatCorrector,
    tage_core::{TageCore, TagePrediction},
};
use crate::core::units::bru::direction::{BranchClass, DirectionPredictor, Retired};

/// SC-L-TAGE + ITTAGE composed predictor.
#[derive(Debug)]
pub struct ScLTagePredictor {
    spec_ghr: Ghr,
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
    /// The speculative global history before the prediction.
    ghr: Ghr,
    /// The TAGE entries a conditional branch's prediction read.
    tage: Option<TagePrediction>,
    /// The TAGE metadata and SC sum the statistical corrector decided
    /// with; `None` when the loop predictor overrode it or for a jump.
    sc: Option<(TageScMeta, ScSum)>,
}

impl ScLTagePredictor {
    /// Creates a new SC-L-TAGE + ITTAGE predictor.
    pub fn new(
        tage_config: &TageConfig,
        sc_config: &ScConfig,
        ittage_config: &IttageConfig,
    ) -> Self {
        let tage = TageCore::new(tage_config);
        let max_hist = tage.max_history();

        Self {
            spec_ghr: Ghr::with_len(max_hist),
            commit_ghr: Ghr::with_len(max_hist),
            tage,
            loop_pred: LoopPredictor::new(tage_config.loop_table_size),
            sc: StatCorrector::new(sc_config),
            ittage: Ittage::new(ittage_config),
        }
    }

    fn push_speculative(&mut self, taken: bool) {
        self.tage.speculate(taken, &self.spec_ghr);
        self.ittage.speculate(taken, &self.spec_ghr);
        self.spec_ghr.push(taken);
    }

    fn repair_speculative(&mut self) {
        self.tage.repair(&self.spec_ghr);
        self.ittage.repair_history(&self.spec_ghr);
    }

    /// Advances the committed history. Must follow every read of the
    /// committed CSRs for this instruction: the folded histories need the
    /// bit about to be shifted out.
    fn push_committed(&mut self, taken: bool) {
        self.ittage.commit_advance(taken, &self.commit_ghr);
        self.commit_ghr.push(taken);
    }

    fn train_direction(&mut self, pc: u64, taken: bool, history: &ScLTageHistory) {
        self.loop_pred.update(pc, taken);
        let Some(prediction) = &history.tage else { return };
        self.tage.update(taken, prediction);
        let meta = prediction.meta();
        let (sc_meta, sc_sum) = history.sc.unwrap_or_else(|| {
            let (_taken, sum) = self.sc.predict(pc, &self.commit_ghr, &meta);
            (meta, sum)
        });
        self.sc.update(pc, &self.commit_ghr, taken, &sc_meta, sc_sum);
    }
}

impl DirectionPredictor for ScLTagePredictor {
    type History = ScLTageHistory;

    fn lookup(&self, pc: u64) -> (bool, ScLTageHistory) {
        let prediction = self.tage.predict(pc);
        let tage = Some(prediction);
        if let Some(loop_taken) = self.loop_pred.predict(pc) {
            return (loop_taken, ScLTageHistory { ghr: self.spec_ghr, tage, sc: None });
        }
        let meta = prediction.meta();
        let (sc_taken, sc_sum) = self.sc.predict(pc, &self.spec_ghr, &meta);
        (sc_taken, ScLTageHistory { ghr: self.spec_ghr, tage, sc: Some((meta, sc_sum)) })
    }

    fn unconditional(&self, _pc: u64) -> ScLTageHistory {
        ScLTageHistory { ghr: self.spec_ghr, tage: None, sc: None }
    }

    fn update_histories(&mut self, _pc: u64, taken: bool, _history: &ScLTageHistory) {
        self.push_speculative(taken);
    }

    fn squash(&mut self, history: &ScLTageHistory) {
        self.spec_ghr = history.ghr;
    }

    fn squash_done(&mut self) {
        self.repair_speculative();
    }

    fn correct(&mut self, _pc: u64, taken: bool, history: &ScLTageHistory) {
        self.spec_ghr = history.ghr;
        self.repair_speculative();
        self.push_speculative(taken);
    }

    fn commit(&mut self, pc: u64, retired: Retired, history: &ScLTageHistory) {
        if retired.class == BranchClass::Conditional {
            self.train_direction(pc, retired.taken, history);
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

    fn test_tage_config() -> TageConfig {
        TageConfig {
            num_banks: 4,
            table_size: 256,
            loop_table_size: 16,
            reset_interval: 100_000,
            history_lengths: vec![5, 15, 44, 130],
            tag_widths: vec![9, 9, 10, 10],
        }
    }

    fn test_sc_config() -> ScConfig {
        ScConfig {
            num_tables: 4,
            table_size: 64,
            history_lengths: vec![0, 2, 4, 8],
            counter_bits: 3,
            bias_table_size: 256,
            bias_counter_bits: 6,
            initial_threshold: 35,
            per_pc_threshold_bits: 6,
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
        ScLTagePredictor::new(&test_tage_config(), &test_sc_config(), &test_ittage_config())
    }

    #[test]
    fn an_untrained_branch_is_predicted_taken() {
        let (taken, _) = predictor().lookup(0x8000_1000);
        assert!(taken, "Base counter 0 should predict taken (>= 0)");
    }

    #[test]
    fn squashing_younger_predictions_restores_the_history_they_shifted() {
        let mut pred = predictor();
        for i in 0u64..20 {
            let history = pred.unconditional(0x1000 + i * 4);
            pred.update_histories(0x1000 + i * 4, i % 2 == 0, &history);
        }
        let before = pred.spec_ghr;
        let mut squashed = Vec::new();
        for _ in 0..10 {
            let (_, history) = pred.lookup(0x2000);
            pred.update_histories(0x2000, true, &history);
            squashed.push(history);
        }

        for history in squashed.iter().rev() {
            pred.squash(history);
        }
        pred.squash_done();

        assert_eq!(pred.spec_ghr, before);
    }

    #[test]
    fn a_committed_indirect_jump_teaches_ittage_its_target() {
        let mut pred = predictor();
        let pc = 0x8000_2000u64;
        let target = 0x8000_5000u64;
        let history = pred.unconditional(pc);

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
