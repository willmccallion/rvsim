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
//! 4. Target: ITTAGE for indirect branches, BTB otherwise

use std::cell::Cell;

use crate::config::{IttageConfig, ScConfig, TageConfig};
use crate::core::units::bru::{
    BranchPredictor, Ghr,
    btb::Btb,
    components::{
        ittage::Ittage, loop_predictor::LoopPredictor, sc_types::ScSum,
        stat_corrector::StatCorrector, tage_core::TageCore,
    },
    ras::Ras,
};

/// Number of entries in the predict-time SC metadata cache.
const SC_CACHE_SIZE: usize = 64;
const SC_CACHE_MASK: usize = SC_CACHE_SIZE - 1;

/// Cached predict-time SC metadata for a single PC.
#[derive(Clone, Copy, Debug, Default)]
struct ScCacheEntry {
    pc: u64,
    meta: Option<(crate::core::units::bru::components::sc_types::TageScMeta, ScSum)>,
}

/// SC-L-TAGE + ITTAGE composed predictor.
#[derive(Debug)]
pub struct ScLTagePredictor {
    btb: Btb,
    ras: Ras,
    spec_ghr: Ghr,
    commit_ghr: Ghr,

    /// Shared TAGE direction core.
    tage: TageCore,

    // Composable components
    loop_pred: LoopPredictor,
    sc: StatCorrector,
    ittage: Ittage,

    /// Direct-mapped cache of predict-time SC metadata.
    sc_cache: Vec<Cell<ScCacheEntry>>,
}

impl ScLTagePredictor {
    /// Creates a new SC-L-TAGE + ITTAGE predictor.
    pub fn new(
        tage_config: &TageConfig,
        sc_config: &ScConfig,
        ittage_config: &IttageConfig,
        btb_size: usize,
        btb_ways: usize,
        ras_size: usize,
    ) -> Self {
        let tage = TageCore::new(tage_config);
        let max_hist = tage.max_history();

        Self {
            btb: Btb::new(btb_size, btb_ways),
            ras: Ras::new(ras_size),
            spec_ghr: Ghr::with_len(max_hist),
            commit_ghr: Ghr::with_len(max_hist),
            tage,
            loop_pred: LoopPredictor::new(tage_config.loop_table_size),
            sc: StatCorrector::new(sc_config),
            ittage: Ittage::new(ittage_config),
            sc_cache: vec![Cell::new(ScCacheEntry::default()); SC_CACHE_SIZE],
        }
    }
}

impl BranchPredictor for ScLTagePredictor {
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        // 1. TAGE base prediction.
        let meta = self.tage.predict(pc);

        // 2. Loop predictor override.
        if let Some(loop_taken) = self.loop_pred.predict(pc) {
            return (loop_taken, if loop_taken { self.btb.lookup(pc) } else { None });
        }

        // 3. SC correction.
        let (sc_taken, sc_sum) = self.sc.predict(pc, &self.spec_ghr, &meta);

        // Cache predict-time SC metadata for use at update time.
        let cache_idx = (pc >> 2) as usize & SC_CACHE_MASK;
        self.sc_cache[cache_idx].set(ScCacheEntry { pc, meta: Some((meta, sc_sum)) });

        (sc_taken, if sc_taken { self.btb.lookup(pc) } else { None })
    }

    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>, ghr_snapshot: &Ghr) {
        self.loop_pred.update(pc, taken);

        // TAGE direction update — use committed GHR (actual outcomes) to match
        // Seznec's CBP functional model.
        let result = self.tage.update(pc, taken, &self.commit_ghr);
        let meta = result.meta;

        // Update SC: use cached predict-time metadata if available, else reconstruct.
        let cache_idx = (pc >> 2) as usize & SC_CACHE_MASK;
        let cached = self.sc_cache[cache_idx].get();
        let (sc_meta, sc_sum) = if let Some((m, s)) = cached.meta.filter(|_| cached.pc == pc) {
            (m, s)
        } else {
            let (_p, s) = self.sc.predict(pc, &self.commit_ghr, &meta);
            (meta, s)
        };
        self.sc.update(pc, &self.commit_ghr, taken, &sc_meta, sc_sum);

        // Update ITTAGE for indirect branches.
        if let Some(tgt) = target {
            self.ittage.update(pc, tgt, ghr_snapshot);
            self.btb.update(pc, tgt);
        }

        self.commit_ghr.push(taken);
    }

    fn predict_btb(&self, pc: u64) -> Option<u64> {
        self.ittage.predict(pc).or_else(|| self.btb.lookup(pc))
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
        self.ittage.speculate(taken, &self.spec_ghr);
        self.spec_ghr.push(taken);
    }

    fn snapshot_history(&self) -> Ghr {
        self.spec_ghr
    }

    fn repair_history(&mut self, ghr: &Ghr) {
        self.spec_ghr = *ghr;
        self.tage.repair(&self.spec_ghr);
        self.ittage.repair_history(ghr);
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
        self.ittage.repair_to_committed(&self.commit_ghr);
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

    #[test]
    fn test_sc_l_tage_basic_prediction() {
        let pred = ScLTagePredictor::new(
            &test_tage_config(),
            &test_sc_config(),
            &test_ittage_config(),
            64,
            4,
            8,
        );

        let (taken, _target) = pred.predict_branch(0x8000_1000);
        assert!(taken, "Base counter 0 should predict taken (>= 0)");
    }

    #[test]
    fn test_sc_l_tage_speculate_and_repair() {
        let mut pred = ScLTagePredictor::new(
            &test_tage_config(),
            &test_sc_config(),
            &test_ittage_config(),
            64,
            4,
            8,
        );

        for i in 0u64..20 {
            pred.speculate(0x1000 + i * 4, i % 2 == 0);
        }
        let snapshot = pred.snapshot_history();

        for _ in 0..10 {
            pred.speculate(0x2000, true);
        }

        pred.repair_history(&snapshot);
        let restored = pred.snapshot_history();
        assert_eq!(snapshot, restored);
    }

    #[test]
    fn test_sc_l_tage_ittage_indirect() {
        let mut pred = ScLTagePredictor::new(
            &test_tage_config(),
            &test_sc_config(),
            &test_ittage_config(),
            64,
            4,
            8,
        );

        let pc = 0x8000_2000u64;
        let target = 0x8000_5000u64;

        assert_eq!(pred.predict_btb(pc), None);

        let snapshot = pred.snapshot_history();
        pred.update_branch(pc, true, Some(target), &snapshot);

        let btb_result = pred.predict_btb(pc);
        assert_eq!(btb_result, Some(target));
    }
}
