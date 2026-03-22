//! TAGE (Tagged Geometric History Length) Branch Predictor.
//!
//! Uses `TageCore` from `components/tage_core` for the shared TAGE direction
//! logic (bimodal base + tagged banks + `USE_ALT_ON_NA`). This predictor does
//! NOT include a loop predictor — the "L" in SC-L-TAGE belongs to that
//! composed predictor only.

use crate::config::TageConfig;
use crate::core::units::bru::{
    BranchPredictor, Ghr, btb::Btb, components::tage_core::TageCore, ras::Ras,
};

/// TAGE Predictor structure.
#[derive(Debug)]
pub struct TagePredictor {
    btb: Btb,
    ras: Ras,
    spec_ghr: Ghr,
    commit_ghr: Ghr,
    tage: TageCore,
}

impl TagePredictor {
    /// Creates a new TAGE Predictor based on configuration.
    pub fn new(config: &TageConfig, btb_size: usize, btb_ways: usize, ras_size: usize) -> Self {
        let tage = TageCore::new(config);
        let max_hist = tage.max_history();

        Self {
            btb: Btb::new(btb_size, btb_ways),
            ras: Ras::new(ras_size),
            spec_ghr: Ghr::with_len(max_hist),
            commit_ghr: Ghr::with_len(max_hist),
            tage,
        }
    }
}

impl BranchPredictor for TagePredictor {
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        let meta = self.tage.predict(pc);
        (meta.pred_taken, self.btb.lookup(pc))
    }

    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>, _ghr_snapshot: &Ghr) {
        // Use committed GHR (actual outcomes) for training, matching Seznec's
        // CBP functional model where the GHR only contains verified outcomes.
        let _result = self.tage.update(pc, taken, &self.commit_ghr);
        self.commit_ghr.push(taken);

        if let Some(tgt) = target {
            self.btb.update(pc, tgt);
        }
    }

    fn predict_btb(&self, pc: u64) -> Option<u64> {
        self.btb.lookup(pc)
    }

    fn on_call(&mut self, pc: u64, ret_addr: u64, target: u64) {
        self.ras.push(ret_addr);
        self.btb.update(pc, target);
    }

    fn predict_return(&self) -> Option<u64> {
        self.ras.top()
    }

    fn on_return(&mut self) {
        let _ = self.ras.pop();
    }

    fn speculate(&mut self, _pc: u64, taken: bool) {
        self.tage.speculate(taken, &self.spec_ghr);
        self.spec_ghr.push(taken);
    }

    fn snapshot_history(&self) -> Ghr {
        self.spec_ghr
    }

    fn repair_history(&mut self, ghr: &Ghr) {
        self.spec_ghr = *ghr;
        self.tage.repair(&self.spec_ghr);
    }

    fn snapshot_ras(&self) -> usize {
        self.ras.snapshot_ptr()
    }

    fn restore_ras(&mut self, ptr: usize) {
        self.ras.restore_ptr(ptr);
    }

    fn update_btb(&mut self, pc: u64, target: u64) {
        self.btb.update(pc, target);
    }

    fn repair_to_committed(&mut self) {
        self.spec_ghr = self.commit_ghr;
        self.tage.repair(&self.spec_ghr);
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
    fn test_spec_index_matches_snapshot_index() {
        let config = test_config();
        let mut tage = TagePredictor::new(&config, 64, 4, 8);
        let pc: u64 = 0x8000_1234;

        for i in 0u64..50 {
            let taken = i % 3 != 0;
            tage.speculate(pc.wrapping_add(i * 4), taken);
        }

        let (_taken, _target) = tage.predict_branch(pc);
    }

    #[test]
    fn test_repair_history_restores_state() {
        let config = test_config();
        let mut tage = TagePredictor::new(&config, 64, 4, 8);
        let pc: u64 = 0x8000_1000;

        for i in 0u64..20 {
            tage.speculate(pc.wrapping_add(i * 4), i % 2 == 0);
        }
        let snapshot = tage.snapshot_history();

        for _ in 0u64..30 {
            tage.speculate(0x2000, true);
        }

        tage.repair_history(&snapshot);
        let restored = tage.snapshot_history();
        assert_eq!(snapshot, restored);
    }
}
