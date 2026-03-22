//! Core TAGE direction predictor logic shared by standalone TAGE and SC-L-TAGE.
//!
//! Owns the bimodal base table, tagged banks (`GeoBankSet`), and `USE_ALT_ON_NA`
//! counter. Does NOT own a GHR, BTB, RAS, loop predictor, or SC — callers
//! compose those on top.

use super::sc_types::{TageConfLevel, TageScMeta};
use super::tagged_bank::GeoBankSet;
use crate::config::TageConfig;
use crate::core::units::bru::Ghr;

/// An entry in a TAGE tagged bank.
#[derive(Clone, Copy, Debug, Default)]
struct TageEntry {
    tag: u16,
    ctr: i8,
    u: u8,
}

/// Result of a commit-time TAGE update, providing metadata for the SC layer.
#[derive(Clone, Copy, Debug)]
pub struct TageUpdateResult {
    /// Structured metadata (confidence, provider bank, effective prediction).
    pub meta: TageScMeta,
    /// Effective TAGE prediction after `USE_ALT_ON_NA`.
    pub tage_taken: bool,
}

/// Core TAGE direction predictor.
///
/// Provides speculative prediction via `predict()` and commit-time update via
/// `update()`. CSR management is delegated to the internal `GeoBankSet`.
#[derive(Debug)]
pub struct TageCore {
    base: Vec<i8>,
    geo_banks: GeoBankSet,
    tables: Vec<Vec<TageEntry>>,
    use_alt_on_na_ctr: i8,
    clock_counter: u32,
    reset_interval: u32,
}

impl TageCore {
    /// Creates a new TAGE core from config.
    ///
    /// # Panics
    ///
    /// Panics if `table_size` is not a power of two, or if `history_lengths`
    /// and `tag_widths` have different lengths, or max history exceeds 1024.
    pub fn new(config: &TageConfig) -> Self {
        assert!(config.table_size.is_power_of_two(), "TAGE table size must be power of 2");

        let num_banks = config.num_banks;
        let hist_lengths = &config.history_lengths;
        let tag_widths = &config.tag_widths;

        assert_eq!(hist_lengths.len(), num_banks);
        assert_eq!(tag_widths.len(), num_banks);

        let table_bits = config.table_size.trailing_zeros() as usize;
        let max_hist = *hist_lengths.iter().max().unwrap_or(&64);

        assert!(
            max_hist <= 1024,
            "TAGE: max history length {max_hist} exceeds GHR capacity of 1024 bits.",
        );

        let mut tables = Vec::with_capacity(num_banks);
        for _ in 0..num_banks {
            tables.push(vec![TageEntry::default(); config.table_size]);
        }

        let geo_banks = GeoBankSet::new(hist_lengths, tag_widths, table_bits);

        Self {
            base: vec![0; config.table_size],
            geo_banks,
            tables,
            use_alt_on_na_ctr: 0,
            clock_counter: 0,
            reset_interval: config.reset_interval,
        }
    }

    /// Maximum history length needed for the GHR.
    pub fn max_history(&self) -> usize {
        let mut max = 0;
        for i in 0..self.geo_banks.num_banks() {
            let hl = self.geo_banks.hist_length(i);
            if hl > max {
                max = hl;
            }
        }
        max
    }

    /// Table mask for base predictor indexing.
    #[inline]
    pub const fn table_mask(&self) -> usize {
        self.geo_banks.table_mask()
    }

    /// Speculative prediction using pre-maintained CSRs. `O(num_banks)`.
    ///
    /// Returns structured `TageScMeta` with confidence, provider info, and
    /// effective prediction after `USE_ALT_ON_NA`.
    pub fn predict(&self, pc: u64) -> TageScMeta {
        let num_banks = self.tables.len();
        let base_idx = ((pc >> 2) as usize) & self.geo_banks.table_mask();

        let mut provider: Option<usize> = None;
        let mut alt: Option<usize> = None;

        for i in (0..num_banks).rev() {
            let idx = self.geo_banks.spec_index(pc, i);
            let tag = self.geo_banks.spec_tag(pc, i);
            if self.tables[i][idx].tag == tag {
                if provider.is_none() {
                    provider = Some(i);
                } else {
                    alt = Some(i);
                    break;
                }
            }
        }

        let Some(prov_bank) = provider else {
            let ctr = self.base[base_idx];
            return TageScMeta {
                conf: TageConfLevel::from_ctr(ctr),
                provider_bank: 0,
                alt_bank_present: false,
                pred_taken: ctr >= 0,
                pred_ctr: ctr,
            };
        };

        let prov_idx = self.geo_banks.spec_index(pc, prov_bank);
        let prov_ctr = self.tables[prov_bank][prov_idx].ctr;
        let prov_taken = prov_ctr >= 0;

        let provider_weak = prov_ctr == 0 || prov_ctr == -1;
        if provider_weak && self.use_alt_on_na_ctr >= 0 {
            let (alt_ctr, alt_taken) = alt.map_or_else(
                || {
                    let ctr = self.base[base_idx];
                    (ctr, ctr >= 0)
                },
                |bank| {
                    let idx = self.geo_banks.spec_index(pc, bank);
                    let ctr = self.tables[bank][idx].ctr;
                    (ctr, ctr >= 0)
                },
            );
            return TageScMeta {
                conf: TageConfLevel::from_ctr(alt_ctr),
                provider_bank: prov_bank + 1,
                alt_bank_present: alt.is_some(),
                pred_taken: alt_taken,
                pred_ctr: alt_ctr,
            };
        }

        TageScMeta {
            conf: TageConfLevel::from_ctr(prov_ctr),
            provider_bank: prov_bank + 1,
            alt_bank_present: alt.is_some(),
            pred_taken: prov_taken,
            pred_ctr: prov_ctr,
        }
    }

    /// Commit-time update. Recomputes indices/tags from the GHR snapshot,
    /// updates counters, useful bits, `USE_ALT_ON_NA`, and allocates on
    /// misprediction. Returns metadata for SC consumption.
    pub fn update(&mut self, pc: u64, taken: bool, ghr: &Ghr) -> TageUpdateResult {
        // Periodic useful-bit reset.
        self.clock_counter += 1;
        if self.clock_counter >= self.reset_interval {
            self.clock_counter = 0;
            for table in &mut self.tables {
                for entry in table {
                    entry.u >>= 1;
                }
            }
        }

        let (indices, tags) = self.geo_banks.snapshot_all(pc, ghr);
        let num_banks = self.tables.len();

        // Find provider (longest matching) and alt.
        let mut provider = 0usize; // 0 = bimodal, 1+ = bank+1
        let mut alt = 0usize;

        for i in (0..num_banks).rev() {
            if self.tables[i][indices[i]].tag == tags[i] {
                if provider == 0 {
                    provider = i + 1;
                } else if alt == 0 {
                    alt = i + 1;
                    break;
                }
            }
        }

        let base_idx = ((pc >> 2) as usize) & self.geo_banks.table_mask();

        let (prov_taken, prov_ctr) = if provider > 0 {
            let ctr = self.tables[provider - 1][indices[provider - 1]].ctr;
            (ctr >= 0, ctr)
        } else {
            let ctr = self.base[base_idx];
            (ctr >= 0, ctr)
        };

        let alt_taken = if alt > 0 {
            self.tables[alt - 1][indices[alt - 1]].ctr >= 0
        } else {
            self.base[base_idx] >= 0
        };

        // USE_ALT_ON_NA update.
        let provider_weak = provider > 0 && (prov_ctr == 0 || prov_ctr == -1);
        if provider_weak && prov_taken != alt_taken {
            if alt_taken == taken {
                self.use_alt_on_na_ctr = (self.use_alt_on_na_ctr + 1).min(7);
            } else {
                self.use_alt_on_na_ctr = (self.use_alt_on_na_ctr - 1).max(-8);
            }
        }

        // Effective prediction after USE_ALT_ON_NA.
        let (eff_taken, eff_ctr) = if provider_weak && self.use_alt_on_na_ctr >= 0 {
            if alt > 0 {
                let ctr = self.tables[alt - 1][indices[alt - 1]].ctr;
                (ctr >= 0, ctr)
            } else {
                let ctr = self.base[base_idx];
                (ctr >= 0, ctr)
            }
        } else {
            (prov_taken, prov_ctr)
        };

        let tage_taken = eff_taken;
        let provider_mispred = prov_taken != taken;
        let tage_mispred = tage_taken != taken;

        // Update provider counter.
        if provider > 0 {
            let bank_idx = provider - 1;
            let idx = indices[bank_idx];
            let e = &mut self.tables[bank_idx][idx];

            if taken {
                if e.ctr < 3 {
                    e.ctr += 1;
                }
            } else if e.ctr > -4 {
                e.ctr -= 1;
            }

            // Useful bits.
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
            let start_bank = if provider == 0 { 0 } else { provider };

            if start_bank < num_banks {
                let mut allocated = false;
                for (table, (&idx, &tag)) in self.tables[start_bank..num_banks]
                    .iter_mut()
                    .zip(indices[start_bank..num_banks].iter().zip(&tags[start_bank..num_banks]))
                {
                    let e = &mut table[idx];
                    if e.u == 0 {
                        e.tag = tag;
                        e.ctr = if taken { 0 } else { -1 };
                        e.u = 1;
                        allocated = true;
                        break;
                    }
                }

                if !allocated {
                    for (table, &idx) in self.tables[start_bank..num_banks]
                        .iter_mut()
                        .zip(&indices[start_bank..num_banks])
                    {
                        if table[idx].u > 0 {
                            table[idx].u -= 1;
                        }
                    }
                }
            }
        }

        let meta = TageScMeta {
            conf: TageConfLevel::from_ctr(eff_ctr),
            provider_bank: provider,
            alt_bank_present: alt > 0,
            pred_taken: eff_taken,
            pred_ctr: eff_ctr,
        };

        TageUpdateResult { meta, tage_taken }
    }

    /// Incrementally updates CSRs for a new speculative branch outcome.
    /// Must be called BEFORE the caller's `ghr.push()`.
    #[inline]
    pub fn speculate(&mut self, taken: bool, ghr: &Ghr) {
        self.geo_banks.update_csrs(taken, ghr);
    }

    /// Recomputes CSRs from a GHR snapshot (misprediction recovery).
    pub fn repair(&mut self, ghr: &Ghr) {
        self.geo_banks.recompute_all(ghr);
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
    fn test_predict_default_taken() {
        let config = test_config();
        let tage = TageCore::new(&config);
        let meta = tage.predict(0x8000_1000);
        // Default counters are 0, which is >= 0 -> taken.
        assert!(meta.pred_taken);
    }

    #[test]
    fn test_speculate_and_repair() {
        let config = test_config();
        let mut tage = TageCore::new(&config);
        let max_hist = tage.max_history();
        let mut ghr = Ghr::with_len(max_hist);
        let pc = 0x8000_1234u64;

        for i in 0u64..50 {
            let taken = i % 3 != 0;
            tage.speculate(taken, &ghr);
            ghr.push(taken);
        }

        let snapshot = ghr;
        let saved_meta = tage.predict(pc);

        // Diverge.
        for _ in 0..30 {
            tage.speculate(true, &ghr);
            ghr.push(true);
        }

        // Repair.
        ghr = snapshot;
        tage.repair(&ghr);
        let restored_meta = tage.predict(pc);
        assert_eq!(saved_meta.pred_taken, restored_meta.pred_taken);
    }

    #[test]
    fn test_update_trains_predictor() {
        let config = test_config();
        let mut tage = TageCore::new(&config);
        let max_hist = tage.max_history();
        let ghr = Ghr::with_len(max_hist);
        let pc = 0x8000_1000u64;

        // Train not-taken heavily.
        for _ in 0..50 {
            let _ = tage.update(pc, false, &ghr);
        }

        let meta = tage.predict(pc);
        assert!(!meta.pred_taken, "Should predict not-taken after heavy training");
    }
}
