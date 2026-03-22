//! TAGE (Tagged Geometric History Length) Branch Predictor with Loop Predictor.
//!
//! TAGE uses a base bimodal predictor and multiple tagged banks indexed with
//! geometrically increasing history lengths. It provides high accuracy by
//! matching long history patterns while falling back to shorter histories
//! or the base predictor when necessary.
//!
//! This implementation uses shared components from `components/`:
//! - `GeoBankSet` for CSR management and index/tag computation
//! - `LoopPredictor` for loop detection and override

use crate::config::TageConfig;
use crate::core::units::bru::{
    BranchPredictor, Ghr,
    btb::Btb,
    components::{loop_predictor::LoopPredictor, tagged_bank::GeoBankSet},
    ras::Ras,
};

/// An entry in a TAGE bank.
#[derive(Clone, Copy, Debug, Default)]
struct TageEntry {
    /// Tag for matching the history/PC hash.
    tag: u16,
    /// 3-bit saturating counter for prediction.
    ctr: i8,
    /// 2-bit useful counter for replacement policy.
    u: u8,
}

/// TAGE Predictor structure with integrated loop predictor.
#[derive(Debug)]
pub struct TagePredictor {
    /// Branch Target Buffer.
    btb: Btb,
    /// Return Address Stack.
    ras: Ras,

    /// Speculative wide Global History Register.
    spec_ghr: Ghr,
    /// Committed wide Global History Register.
    commit_ghr: Ghr,

    /// Base bimodal predictor table.
    base: Vec<i8>,
    /// Tagged component banks (managed by `GeoBankSet` for CSR/indexing).
    geo_banks: GeoBankSet,
    /// Tagged bank entry storage.
    banks: Vec<Vec<TageEntry>>,

    /// Index of the bank providing the current prediction.
    provider_bank: usize,
    /// Index of the alternative bank.
    alt_bank: usize,

    /// Counter for periodic reset of useful bits.
    clock_counter: u32,
    /// Interval for resetting useful bits.
    reset_interval: u32,

    /// `USE_ALT_ON_NA`: 4-bit counter that learns whether to trust newly
    /// allocated (weak) provider entries or prefer the alt prediction.
    use_alt_on_na_ctr: i8,

    /// Loop predictor component.
    loop_pred: LoopPredictor,
}

impl TagePredictor {
    /// Creates a new TAGE Predictor based on configuration.
    ///
    /// # Panics
    ///
    /// Panics if `config.table_size` is not a power of two, or if `history_lengths`
    /// and `tag_widths` have different lengths.
    pub fn new(config: &TageConfig, btb_size: usize, btb_ways: usize, ras_size: usize) -> Self {
        assert!(config.table_size.is_power_of_two(), "TAGE table size must be power of 2");

        let num_banks = config.num_banks;
        let hist_lengths = &config.history_lengths;
        let tag_widths = &config.tag_widths;

        assert_eq!(
            hist_lengths.len(),
            num_banks,
            "TAGE: history_lengths.len() ({}) must match num_banks ({num_banks})",
            hist_lengths.len(),
        );
        assert_eq!(
            tag_widths.len(),
            num_banks,
            "TAGE: tag_widths.len() ({}) must match num_banks ({num_banks})",
            tag_widths.len(),
        );

        let table_bits = config.table_size.trailing_zeros() as usize;
        let max_hist = *hist_lengths.iter().max().unwrap_or(&64);

        assert!(
            max_hist <= 1024,
            "TAGE: max history length {max_hist} exceeds GHR capacity of 1024 bits. \
             Reduce your history_lengths config.",
        );

        let mut banks = Vec::with_capacity(num_banks);
        for _ in 0..num_banks {
            banks.push(vec![TageEntry::default(); config.table_size]);
        }

        let geo_banks = GeoBankSet::new(hist_lengths, tag_widths, table_bits);

        Self {
            btb: Btb::new(btb_size, btb_ways),
            ras: Ras::new(ras_size),
            spec_ghr: Ghr::with_len(max_hist),
            commit_ghr: Ghr::with_len(max_hist),
            base: vec![0; config.table_size],
            geo_banks,
            banks,

            provider_bank: 0,
            alt_bank: 0,
            clock_counter: 0,
            reset_interval: config.reset_interval,
            use_alt_on_na_ctr: 0,

            loop_pred: LoopPredictor::new(config.loop_table_size),
        }
    }
}

impl BranchPredictor for TagePredictor {
    /// Predicts branch direction and target.
    ///
    /// Searches the tagged banks for the longest history match (provider).
    /// If no match is found, uses the base predictor. The loop predictor
    /// can override when it has high confidence.
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        // Check loop predictor first — high-confidence override.
        if let Some(loop_taken) = self.loop_pred.predict(pc) {
            return (loop_taken, if loop_taken { self.btb.lookup(pc) } else { None });
        }

        let num_banks = self.banks.len();
        let base_idx = ((pc >> 2) as usize) & self.geo_banks.table_mask();

        // Find provider (longest matching) and alt (second longest matching).
        let mut provider: Option<usize> = None;
        let mut alt: Option<usize> = None;

        for i in (0..num_banks).rev() {
            let idx = self.geo_banks.spec_index(pc, i);
            let tag = self.geo_banks.spec_tag(pc, i);
            if self.banks[i][idx].tag == tag {
                if provider.is_none() {
                    provider = Some(i);
                } else {
                    alt = Some(i);
                    break;
                }
            }
        }

        let Some(prov_bank) = provider else {
            return (self.base[base_idx] >= 0, self.btb.lookup(pc));
        };

        let prov_idx = self.geo_banks.spec_index(pc, prov_bank);
        let prov_ctr = self.banks[prov_bank][prov_idx].ctr;

        // USE_ALT_ON_NA: prefer alt when provider is weak (newly allocated).
        let provider_weak = prov_ctr == 0 || prov_ctr == -1;
        if provider_weak && self.use_alt_on_na_ctr >= 0 {
            let alt_taken = alt.map_or_else(
                || self.base[base_idx] >= 0,
                |bank| {
                    let idx = self.geo_banks.spec_index(pc, bank);
                    self.banks[bank][idx].ctr >= 0
                },
            );
            return (alt_taken, self.btb.lookup(pc));
        }

        (prov_ctr >= 0, self.btb.lookup(pc))
    }

    /// Updates the predictor state at commit time.
    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>, ghr_snapshot: &Ghr) {
        // Update loop predictor.
        self.loop_pred.update(pc, taken);

        self.clock_counter += 1;
        if self.clock_counter >= self.reset_interval {
            self.clock_counter = 0;
            for bank in &mut self.banks {
                for entry in bank {
                    entry.u >>= 1;
                }
            }
        }

        let mut provider = 0;
        let mut alt = 0;
        let num_banks = self.banks.len();

        for i in (0..num_banks).rev() {
            let idx = self.geo_banks.snapshot_index(pc, i, ghr_snapshot);
            let tag = self.geo_banks.snapshot_tag(pc, i, ghr_snapshot);
            if self.banks[i][idx].tag == tag {
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
            let ctr = self.banks[self.provider_bank - 1][idx].ctr;
            (ctr >= 0, ctr as i32)
        } else {
            let ctr = self.base[base_idx];
            (ctr >= 0, ctr as i32)
        };

        let alt_taken = if self.alt_bank > 0 {
            let idx = self.geo_banks.snapshot_index(pc, self.alt_bank - 1, ghr_snapshot);
            self.banks[self.alt_bank - 1][idx].ctr >= 0
        } else {
            self.base[base_idx] >= 0
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

        // Effective prediction after USE_ALT_ON_NA (for allocation).
        let tage_taken =
            if provider_weak && self.use_alt_on_na_ctr >= 0 { alt_taken } else { prov_taken };

        let provider_mispred = prov_taken != taken;
        let tage_mispred = tage_taken != taken;

        if self.provider_bank > 0 {
            let bank_idx = self.provider_bank - 1;
            let idx = self.geo_banks.snapshot_index(pc, bank_idx, ghr_snapshot);
            let e = &mut self.banks[bank_idx][idx];

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
                    let e = &mut self.banks[i][idx];

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
                        if self.banks[i][idx].u > 0 {
                            self.banks[i][idx].u -= 1;
                        }
                    }
                }
            }
        }

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
        self.geo_banks.update_csrs(taken, &self.spec_ghr);
        self.spec_ghr.push(taken);
    }

    fn snapshot_history(&self) -> Ghr {
        self.spec_ghr
    }

    fn repair_history(&mut self, ghr: &Ghr) {
        self.spec_ghr = *ghr;
        self.geo_banks.recompute_all(&self.spec_ghr);
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

        let snapshot = tage.snapshot_history();

        for bank in 0..config.num_banks {
            let si = tage.geo_banks.spec_index(pc, bank);
            let ri = tage.geo_banks.snapshot_index(pc, bank, &snapshot);
            assert_eq!(si, ri, "Index mismatch for bank {bank}");

            let st = tage.geo_banks.spec_tag(pc, bank);
            let rt = tage.geo_banks.snapshot_tag(pc, bank, &snapshot);
            assert_eq!(st, rt, "Tag mismatch for bank {bank}");
        }
    }

    #[test]
    fn test_repair_history_restores_csrs() {
        let config = test_config();
        let mut tage = TagePredictor::new(&config, 64, 4, 8);
        let pc: u64 = 0x8000_1000;

        for i in 0u64..20 {
            tage.speculate(pc.wrapping_add(i * 4), i % 2 == 0);
        }
        let snapshot = tage.snapshot_history();

        // Save spec indices at snapshot point.
        let saved: Vec<usize> =
            (0..config.num_banks).map(|b| tage.geo_banks.spec_index(pc, b)).collect();

        // Diverge.
        for i in 0u64..30 {
            tage.speculate(pc.wrapping_add((20 + i) * 4), true);
        }

        // Repair.
        tage.repair_history(&snapshot);

        let restored: Vec<usize> =
            (0..config.num_banks).map(|b| tage.geo_banks.spec_index(pc, b)).collect();
        assert_eq!(saved, restored, "CSRs not restored correctly");
    }
}
