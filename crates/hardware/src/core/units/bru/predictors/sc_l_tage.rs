//! SC-L-TAGE + ITTAGE Composed Branch Predictor.
//!
//! Combines TAGE (direction), Loop Predictor (counted loop override),
//! Statistical Corrector (correction layer), and ITTAGE (indirect targets)
//! into a single high-accuracy predictor. This is the "best possible"
//! composed predictor following Seznec's CBP-winning designs.
//!
//! Prediction flow:
//! 1. TAGE base → (direction, confidence)
//! 2. Loop predictor override → if confident, use loop prediction
//! 3. SC correction → may flip direction if confident base is wrong
//! 4. Target: ITTAGE for indirect branches, BTB otherwise

use crate::config::{IttageConfig, ScConfig, TageConfig};
use crate::core::units::bru::{
    BranchPredictor, Ghr,
    btb::Btb,
    components::{
        ittage::Ittage, loop_predictor::LoopPredictor, stat_corrector::StatCorrector,
        tagged_bank::GeoBankSet,
    },
    ras::Ras,
};

/// An entry in a TAGE bank (same as standalone TAGE).
#[derive(Clone, Copy, Debug, Default)]
struct TageEntry {
    tag: u16,
    ctr: i8,
    u: u8,
}

/// SC-L-TAGE + ITTAGE composed predictor.
#[derive(Debug)]
pub struct ScLTagePredictor {
    // Infrastructure
    btb: Btb,
    ras: Ras,

    // Core TAGE (direction)
    spec_ghr: Ghr,
    commit_ghr: Ghr,
    base: Vec<i8>,
    geo_banks: GeoBankSet,
    tage_tables: Vec<Vec<TageEntry>>,
    provider_bank: usize,
    alt_bank: usize,
    clock_counter: u32,
    reset_interval: u32,

    // `USE_ALT_ON_NA`: 4-bit counter that learns whether to trust newly
    // allocated (weak) provider entries or prefer the alt prediction.
    use_alt_on_na_ctr: i8,

    // Composable components
    loop_pred: LoopPredictor,
    sc: StatCorrector,
    ittage: Ittage,
}

impl ScLTagePredictor {
    /// Creates a new SC-L-TAGE + ITTAGE predictor.
    ///
    /// # Panics
    ///
    /// Panics if `table_size` is not a power of two, or if `history_lengths`
    /// and `tag_widths` have different lengths, or max history exceeds 1024.
    pub fn new(
        tage_config: &TageConfig,
        sc_config: &ScConfig,
        ittage_config: &IttageConfig,
        btb_size: usize,
        btb_ways: usize,
        ras_size: usize,
    ) -> Self {
        assert!(tage_config.table_size.is_power_of_two(), "TAGE table size must be power of 2");

        let num_banks = tage_config.num_banks;
        let hist_lengths = &tage_config.history_lengths;
        let tag_widths = &tage_config.tag_widths;

        assert_eq!(hist_lengths.len(), num_banks);
        assert_eq!(tag_widths.len(), num_banks);

        let table_bits = tage_config.table_size.trailing_zeros() as usize;
        let max_hist = *hist_lengths.iter().max().unwrap_or(&64);

        assert!(
            max_hist <= 1024,
            "TAGE: max history length {max_hist} exceeds GHR capacity of 1024 bits.",
        );

        let mut tage_tables = Vec::with_capacity(num_banks);
        for _ in 0..num_banks {
            tage_tables.push(vec![TageEntry::default(); tage_config.table_size]);
        }

        let geo_banks = GeoBankSet::new(hist_lengths, tag_widths, table_bits);

        Self {
            btb: Btb::new(btb_size, btb_ways),
            ras: Ras::new(ras_size),
            spec_ghr: Ghr::with_len(max_hist),
            commit_ghr: Ghr::with_len(max_hist),
            base: vec![0; tage_config.table_size],
            geo_banks,
            tage_tables,
            provider_bank: 0,
            alt_bank: 0,
            clock_counter: 0,
            reset_interval: tage_config.reset_interval,
            use_alt_on_na_ctr: 0,
            loop_pred: LoopPredictor::new(tage_config.loop_table_size),
            sc: StatCorrector::new(sc_config),
            ittage: Ittage::new(ittage_config),
        }
    }

    /// Core TAGE prediction with `USE_ALT_ON_NA`: find provider and alt banks,
    /// prefer alt when the provider entry is newly allocated (weak counter)
    /// and the meta-counter favors alt. Returns (taken, confidence).
    fn tage_predict(&self, pc: u64) -> (bool, i32) {
        let num_banks = self.tage_tables.len();
        let base_idx = ((pc >> 2) as usize) & self.geo_banks.table_mask();

        // Find provider (longest matching) and alt (second longest matching).
        let mut provider: Option<usize> = None;
        let mut alt: Option<usize> = None;

        for i in (0..num_banks).rev() {
            let idx = self.geo_banks.spec_index(pc, i);
            let tag = self.geo_banks.spec_tag(pc, i);
            if self.tage_tables[i][idx].tag == tag {
                if provider.is_none() {
                    provider = Some(i);
                } else {
                    alt = Some(i);
                    break;
                }
            }
        }

        // No tagged bank hit — use bimodal base table.
        let Some(prov_bank) = provider else {
            let ctr = self.base[base_idx];
            return (ctr >= 0, ctr as i32);
        };

        let prov_idx = self.geo_banks.spec_index(pc, prov_bank);
        let prov_ctr = self.tage_tables[prov_bank][prov_idx].ctr;
        let prov_taken = prov_ctr >= 0;
        let prov_conf = prov_ctr as i32;

        // Provider is weak (newly allocated) — consider using alt instead.
        let provider_weak = prov_ctr == 0 || prov_ctr == -1;
        if provider_weak && self.use_alt_on_na_ctr >= 0 {
            let (alt_taken, alt_conf) = alt.map_or_else(
                || {
                    let ctr = self.base[base_idx];
                    (ctr >= 0, ctr as i32)
                },
                |bank| {
                    let idx = self.geo_banks.spec_index(pc, bank);
                    let ctr = self.tage_tables[bank][idx].ctr;
                    (ctr >= 0, ctr as i32)
                },
            );
            return (alt_taken, alt_conf);
        }

        (prov_taken, prov_conf)
    }
}

impl BranchPredictor for ScLTagePredictor {
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        // 1. TAGE base prediction.
        let (tage_taken, tage_confidence) = self.tage_predict(pc);

        // 2. Loop predictor override.
        if let Some(loop_taken) = self.loop_pred.predict(pc) {
            return (loop_taken, if loop_taken { self.btb.lookup(pc) } else { None });
        }

        // 3. SC correction.
        let (sc_taken, _sc_sum) = self.sc.predict(pc, &self.spec_ghr, tage_taken, tage_confidence);

        (sc_taken, if sc_taken { self.btb.lookup(pc) } else { None })
    }

    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>, ghr_snapshot: &Ghr) {
        // Update loop predictor.
        self.loop_pred.update(pc, taken);

        // --- TAGE direction update (same logic as standalone TAGE) ---
        self.clock_counter += 1;
        if self.clock_counter >= self.reset_interval {
            self.clock_counter = 0;
            for table in &mut self.tage_tables {
                for entry in table {
                    entry.u >>= 1;
                }
            }
        }

        let num_banks = self.tage_tables.len();
        let mut provider = 0;
        let mut alt = 0;

        for i in (0..num_banks).rev() {
            let idx = self.geo_banks.snapshot_index(pc, i, ghr_snapshot);
            let tag = self.geo_banks.snapshot_tag(pc, i, ghr_snapshot);
            if self.tage_tables[i][idx].tag == tag {
                if provider == 0 {
                    provider = i + 1;
                } else if alt == 0 {
                    alt = i + 1;
                    break;
                }
            }
        }
        self.provider_bank = provider;
        self.alt_bank = alt;

        let base_idx = ((pc >> 2) as usize) & self.geo_banks.table_mask();

        let (prov_taken, prov_confidence) = if self.provider_bank > 0 {
            let idx = self.geo_banks.snapshot_index(pc, self.provider_bank - 1, ghr_snapshot);
            let ctr = self.tage_tables[self.provider_bank - 1][idx].ctr;
            (ctr >= 0, ctr as i32)
        } else {
            let ctr = self.base[base_idx];
            (ctr >= 0, ctr as i32)
        };

        let (alt_taken, alt_confidence) = if self.alt_bank > 0 {
            let idx = self.geo_banks.snapshot_index(pc, self.alt_bank - 1, ghr_snapshot);
            let ctr = self.tage_tables[self.alt_bank - 1][idx].ctr;
            (ctr >= 0, ctr as i32)
        } else {
            let ctr = self.base[base_idx];
            (ctr >= 0, ctr as i32)
        };

        // USE_ALT_ON_NA: update meta-counter when provider is weak.
        let provider_weak =
            self.provider_bank > 0 && (prov_confidence == 0 || prov_confidence == -1);
        if provider_weak && prov_taken != alt_taken {
            if alt_taken == taken {
                self.use_alt_on_na_ctr = (self.use_alt_on_na_ctr + 1).min(7);
            } else {
                self.use_alt_on_na_ctr = (self.use_alt_on_na_ctr - 1).max(-8);
            }
        }

        // Effective TAGE prediction after USE_ALT_ON_NA (for SC and allocation).
        let (tage_taken, tage_confidence) = if provider_weak && self.use_alt_on_na_ctr >= 0 {
            (alt_taken, alt_confidence)
        } else {
            (prov_taken, prov_confidence)
        };

        let provider_mispred = prov_taken != taken;
        let tage_mispred = tage_taken != taken;

        // Update provider counter and useful bits.
        if self.provider_bank > 0 {
            let bank_idx = self.provider_bank - 1;
            let idx = self.geo_banks.snapshot_index(pc, bank_idx, ghr_snapshot);
            let e = &mut self.tage_tables[bank_idx][idx];

            if taken {
                if e.ctr < 3 {
                    e.ctr += 1;
                }
            } else if e.ctr > -4 {
                e.ctr -= 1;
            }

            // Useful bits track whether provider was better than alt.
            if !provider_mispred && (alt_taken != taken) && e.u < 3 {
                e.u += 1;
            }
            if provider_mispred && e.u > 0 {
                e.u -= 1;
            }
        } else {
            let b = &mut self.base[base_idx];
            if taken {
                if *b < 1 {
                    *b += 1;
                }
            } else if *b > -2 {
                *b -= 1;
            }
        }

        // Allocate on effective TAGE misprediction.
        if tage_mispred {
            let start_bank = if self.provider_bank == 0 { 0 } else { self.provider_bank };

            if start_bank < num_banks {
                let mut allocated = false;
                for i in start_bank..num_banks {
                    let idx = self.geo_banks.snapshot_index(pc, i, ghr_snapshot);
                    let tag = self.geo_banks.snapshot_tag(pc, i, ghr_snapshot);
                    let e = &mut self.tage_tables[i][idx];

                    if e.u == 0 {
                        e.tag = tag;
                        e.ctr = if taken { 0 } else { -1 };
                        e.u = 1;
                        allocated = true;
                        break;
                    }
                }

                if !allocated {
                    for i in start_bank..num_banks {
                        let idx = self.geo_banks.snapshot_index(pc, i, ghr_snapshot);
                        if self.tage_tables[i][idx].u > 0 {
                            self.tage_tables[i][idx].u -= 1;
                        }
                    }
                }
            }
        }

        // Update SC using the effective TAGE prediction (after USE_ALT_ON_NA).
        let (_sc_pred, sc_sum) = self.sc.predict(pc, ghr_snapshot, tage_taken, tage_confidence);
        self.sc.update(pc, ghr_snapshot, taken, tage_taken, sc_sum);

        // Update ITTAGE for indirect branches (when target is present).
        if let Some(tgt) = target {
            self.ittage.update(pc, tgt, ghr_snapshot);
            self.btb.update(pc, tgt);
        }

        self.commit_ghr.push(taken);
    }

    fn predict_btb(&self, pc: u64) -> Option<u64> {
        // Try ITTAGE first for indirect branch targets, fall back to BTB.
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
        self.geo_banks.update_csrs(taken, &self.spec_ghr);
        self.ittage.speculate(taken, &self.spec_ghr);
        self.spec_ghr.push(taken);
    }

    fn snapshot_history(&self) -> Ghr {
        self.spec_ghr
    }

    fn repair_history(&mut self, ghr: &Ghr) {
        self.spec_ghr = *ghr;
        self.geo_banks.recompute_all(&self.spec_ghr);
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
        self.geo_banks.recompute_all(&self.spec_ghr);
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

        // Default prediction should be not-taken (all counters at 0).
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

        // Speculate some branches.
        for i in 0u64..20 {
            pred.speculate(0x1000 + i * 4, i % 2 == 0);
        }
        let snapshot = pred.snapshot_history();

        // Diverge.
        for _ in 0..10 {
            pred.speculate(0x2000, true);
        }

        // Repair.
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

        // Initially no ITTAGE prediction, BTB also empty.
        assert_eq!(pred.predict_btb(pc), None);

        // Train via update_branch with a target.
        let snapshot = pred.snapshot_history();
        pred.update_branch(pc, true, Some(target), &snapshot);

        // Now predict_btb should return the target (via ITTAGE or BTB).
        let btb_result = pred.predict_btb(pc);
        assert_eq!(btb_result, Some(target));
    }
}
