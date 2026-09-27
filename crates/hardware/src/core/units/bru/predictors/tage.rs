//! TAGE (Tagged Geometric History Length) Branch Predictor.
//!
//! Uses `TageCore` from `components/tage_core` for the shared TAGE direction
//! logic (bimodal base + tagged banks + `USE_ALT_ON_NA`). This predictor does
//! NOT include a loop predictor — the "L" in SC-L-TAGE belongs to that
//! composed predictor only.

use crate::config::TageConfig;
use crate::core::units::bru::Ghr;
use crate::core::units::bru::components::tage_core::TageCore;
use crate::core::units::bru::direction::{BranchClass, DirectionPredictor, Retired};

/// TAGE Predictor structure.
#[derive(Debug)]
pub struct TagePredictor {
    spec_ghr: Ghr,
    commit_ghr: Ghr,
    tage: TageCore,
}

/// The speculative global history a TAGE prediction was made with.
#[derive(Clone, Copy, Debug)]
pub struct TageHistory {
    ghr: Ghr,
}

impl TagePredictor {
    /// Creates a new TAGE Predictor based on configuration.
    pub fn new(config: &TageConfig) -> Self {
        let tage = TageCore::new(config);
        let max_hist = tage.max_history();

        Self { spec_ghr: Ghr::with_len(max_hist), commit_ghr: Ghr::with_len(max_hist), tage }
    }

    fn push_speculative(&mut self, taken: bool) {
        self.tage.speculate(taken, &self.spec_ghr);
        self.spec_ghr.push(taken);
    }

    fn push_committed(&mut self, taken: bool) {
        self.tage.commit_advance(taken, &self.commit_ghr);
        self.commit_ghr.push(taken);
    }
}

impl DirectionPredictor for TagePredictor {
    type History = TageHistory;

    fn lookup(&self, pc: u64) -> (bool, TageHistory) {
        (self.tage.predict(pc).pred_taken, TageHistory { ghr: self.spec_ghr })
    }

    fn unconditional(&self, _pc: u64) -> TageHistory {
        TageHistory { ghr: self.spec_ghr }
    }

    fn update_histories(&mut self, _pc: u64, taken: bool, _history: &TageHistory) {
        self.push_speculative(taken);
    }

    fn squash(&mut self, history: &TageHistory) {
        self.spec_ghr = history.ghr;
    }

    fn squash_done(&mut self) {
        self.tage.repair(&self.spec_ghr);
    }

    fn correct(&mut self, _pc: u64, taken: bool, history: &TageHistory) {
        self.spec_ghr = history.ghr;
        self.tage.repair(&self.spec_ghr);
        self.push_speculative(taken);
    }

    /// Trains with the committed history, as Seznec's CBP functional model
    /// does, then shifts the outcome into it.
    fn commit(&mut self, pc: u64, retired: Retired, _history: &TageHistory) {
        if retired.class == BranchClass::Conditional {
            let _result = self.tage.update(pc, retired.taken, &self.commit_ghr);
        }
        self.push_committed(retired.taken);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> TageConfig {
        TageConfig {
            num_banks: 4,
            table_size: 256,
            loop_table_size: 16,
            reset_interval: 100_000,
            history_lengths: vec![5, 15, 44, 130],
            tag_widths: vec![9, 9, 10, 10],
        }
    }

    #[test]
    fn squashing_younger_predictions_restores_the_history_they_shifted() {
        let mut tage = TagePredictor::new(&test_config());
        for i in 0u64..20 {
            let (_, history) = tage.lookup(0x8000_1000 + i * 4);
            tage.update_histories(0, i % 2 == 0, &history);
        }
        let before = tage.spec_ghr;

        let mut squashed = Vec::new();
        for _ in 0..30 {
            let (_, history) = tage.lookup(0x2000);
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
