//! TAGE (Tagged Geometric History Length) Branch Predictor.
//!
//! Uses `TageCore` from `components/tage_core` for the shared TAGE direction
//! logic (bimodal base + tagged banks + `USE_ALT_ON_NA`). This predictor does
//! NOT include a loop predictor — the "L" in SC-L-TAGE belongs to that
//! composed predictor only.

use crate::config::TageConfig;
use crate::core::units::bru::Ghr;
use crate::core::units::bru::components::tage_core::{TageCore, TagePrediction};
use crate::core::units::bru::direction::{BranchClass, DirectionPredictor, Retired};

/// TAGE Predictor structure.
#[derive(Debug)]
pub struct TagePredictor {
    spec_ghr: Ghr,
    /// The path history a squash returns to.
    spec_path: u16,
    tage: TageCore,
}

/// The speculative global history a TAGE prediction was made with, and for
/// a conditional branch the entries it read.
#[derive(Clone, Copy, Debug)]
pub struct TageHistory {
    ghr: Ghr,
    path: u16,
    prediction: Option<TagePrediction>,
}

impl TagePredictor {
    /// Creates a new TAGE Predictor based on configuration.
    pub fn new(config: &TageConfig) -> Self {
        let tage = TageCore::new(config);
        let max_hist = tage.max_history();

        Self { spec_ghr: Ghr::with_len(max_hist), spec_path: 0, tage }
    }

    fn push_speculative(&mut self, pc: u64, taken: bool) {
        self.tage.speculate(pc, taken, &self.spec_ghr);
        self.spec_ghr.push(taken);
        self.spec_path = self.tage.path_history();
    }

    const fn snapshot(&self, prediction: Option<TagePrediction>) -> TageHistory {
        TageHistory { ghr: self.spec_ghr, path: self.spec_path, prediction }
    }
}

impl DirectionPredictor for TagePredictor {
    type History = TageHistory;

    fn lookup(&self, pc: u64, _target: u64) -> (bool, TageHistory) {
        let prediction = self.tage.predict(pc);
        (prediction.taken(), self.snapshot(Some(prediction)))
    }

    fn unconditional(&self, _pc: u64) -> TageHistory {
        self.snapshot(None)
    }

    fn update_histories(&mut self, pc: u64, taken: bool, _history: &TageHistory) {
        self.push_speculative(pc, taken);
    }

    fn squash(&mut self, history: &TageHistory) {
        self.spec_ghr = history.ghr;
        self.spec_path = history.path;
    }

    fn squash_done(&mut self) {
        self.tage.repair(&self.spec_ghr, self.spec_path);
    }

    fn correct(&mut self, pc: u64, taken: bool, history: &TageHistory) {
        self.spec_ghr = history.ghr;
        self.spec_path = history.path;
        self.tage.repair(&self.spec_ghr, self.spec_path);
        self.push_speculative(pc, taken);
    }

    /// Trains the entries the prediction read, as gem5 trains those its
    /// `BranchInfo` recorded.
    fn commit(&mut self, _pc: u64, retired: Retired, history: &TageHistory) {
        if retired.class == BranchClass::Conditional
            && let Some(prediction) = &history.prediction
        {
            self.tage.update(retired.taken, prediction);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> TageConfig {
        TageConfig {
            num_banks: 4,
            table_size: 256,
            reset_interval: 100_000,
            history_lengths: vec![5, 15, 44, 130],
            tag_widths: vec![9, 9, 10, 10],
            ..TageConfig::default()
        }
    }

    #[test]
    fn squashing_younger_predictions_restores_the_history_they_shifted() {
        let mut tage = TagePredictor::new(&test_config());
        for i in 0u64..20 {
            let (_, history) = tage.lookup(0x8000_1000 + i * 4, 0);
            tage.update_histories(0, i % 2 == 0, &history);
        }
        let before = tage.spec_ghr;

        let mut squashed = Vec::new();
        for _ in 0..30 {
            let (_, history) = tage.lookup(0x2000, 0);
            tage.update_histories(0x2000, true, &history);
            squashed.push(history);
        }
        for history in squashed.iter().rev() {
            tage.squash(history);
        }
        tage.squash_done();

        assert_eq!(tage.spec_ghr, before);
    }
}
