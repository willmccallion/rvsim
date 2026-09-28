//! Core TAGE direction predictor logic shared by standalone TAGE and SC-L-TAGE.
//!
//! Owns the bimodal base table, tagged banks (`GeoBankSet`), and `USE_ALT_ON_NA`
//! counter. Does NOT own a GHR, BTB, RAS, loop predictor, or SC — callers
//! compose those on top.

use super::sc_types::{TageConfLevel, TageScMeta};
use super::tagged_bank::{GeoBankSet, MAX_BANKS};
use crate::config::TageConfig;
use crate::core::units::bru::Ghr;

/// An entry in a TAGE tagged bank.
#[derive(Clone, Copy, Debug, Default)]
struct TageEntry {
    tag: u16,
    ctr: i8,
    u: u8,
}

/// What a TAGE prediction read: the entries its branch trains at commit,
/// as gem5's `TAGEBase::BranchInfo` carries them.
#[derive(Clone, Copy, Debug)]
pub struct TagePrediction {
    indices: [usize; MAX_BANKS],
    tags: [u16; MAX_BANKS],
    base_index: usize,
    /// Bank of the longest matching entry.
    provider: Option<usize>,
    /// The longest match's prediction, or the base's without one.
    provider_taken: bool,
    /// The next longest match's prediction, or the base's without one.
    alt_taken: bool,
    /// The longest match's counter is weak, as a new entry's is.
    provider_weak: bool,
    meta: TageScMeta,
}

impl TagePrediction {
    /// The predicted direction, after `USE_ALT_ON_NA`.
    #[must_use]
    pub const fn taken(&self) -> bool {
        self.meta.pred_taken
    }

    /// The metadata the statistical corrector decides with.
    #[must_use]
    pub const fn meta(&self) -> TageScMeta {
        self.meta
    }
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

    /// Predicts the branch at `pc` from the speculative history, recording
    /// the entries it read. `O(num_banks)`.
    pub fn predict(&self, pc: u64) -> TagePrediction {
        let num_banks = self.tables.len();
        let mut indices = [0usize; MAX_BANKS];
        let mut tags = [0u16; MAX_BANKS];
        for bank in 0..num_banks {
            indices[bank] = self.geo_banks.spec_index(pc, bank);
            tags[bank] = self.geo_banks.spec_tag(pc, bank);
        }
        let base_index = ((pc >> 2) as usize) & self.geo_banks.table_mask();

        let mut matching =
            (0..num_banks).rev().filter(|&b| self.tables[b][indices[b]].tag == tags[b]);
        let provider = matching.next();
        let alt = matching.next();

        let base_ctr = self.base[base_index];
        let ctr_of =
            |bank: Option<usize>| bank.map_or(base_ctr, |b| self.tables[b][indices[b]].ctr);
        let (provider_ctr, alt_ctr) = (ctr_of(provider), ctr_of(alt));
        let provider_weak = provider.is_some() && (provider_ctr == 0 || provider_ctr == -1);
        let pred_ctr =
            if provider_weak && self.use_alt_on_na_ctr >= 0 { alt_ctr } else { provider_ctr };
        let meta = TageScMeta {
            conf: TageConfLevel::from_ctr(pred_ctr),
            provider_bank: provider.map_or(0, |b| b + 1),
            alt_bank_present: alt.is_some(),
            pred_taken: pred_ctr >= 0,
            pred_ctr,
        };
        TagePrediction {
            indices,
            tags,
            base_index,
            provider,
            provider_taken: provider_ctr >= 0,
            alt_taken: alt_ctr >= 0,
            provider_weak,
            meta,
        }
    }

    /// Trains the entries `prediction` read with the branch's outcome, and
    /// allocates a longer-history entry when it was wrong.
    pub fn update(&mut self, taken: bool, prediction: &TagePrediction) {
        self.clock_counter += 1;
        if self.clock_counter >= self.reset_interval {
            self.clock_counter = 0;
            for table in &mut self.tables {
                for entry in table {
                    entry.u >>= 1;
                }
            }
        }

        let TagePrediction { indices, tags, base_index, provider, .. } = *prediction;
        let (prov_taken, alt_taken) = (prediction.provider_taken, prediction.alt_taken);
        if prediction.provider_weak && prov_taken != alt_taken {
            if alt_taken == taken {
                self.use_alt_on_na_ctr = (self.use_alt_on_na_ctr + 1).min(7);
            } else {
                self.use_alt_on_na_ctr = (self.use_alt_on_na_ctr - 1).max(-8);
            }
        }
        let num_banks = self.tables.len();
        let provider_mispred = prov_taken != taken;
        let tage_mispred = prediction.taken() != taken;

        if let Some(bank) = provider {
            let e = &mut self.tables[bank][indices[bank]];
            if taken {
                if e.ctr < 3 {
                    e.ctr += 1;
                }
            } else if e.ctr > -4 {
                e.ctr -= 1;
            }
            if !provider_mispred && (alt_taken != taken) && e.u < 3 {
                e.u += 1;
            }
            if provider_mispred && e.u > 0 {
                e.u -= 1;
            }
        } else {
            let b = &mut self.base[base_index];
            if taken {
                if *b < 1 {
                    *b += 1;
                }
            } else if *b > -2 {
                *b -= 1;
            }
        }

        if tage_mispred {
            let start_bank = provider.map_or(0, |bank| bank + 1);
            if start_bank < num_banks {
                let mut allocated = false;
                for bank in start_bank..num_banks {
                    let e = &mut self.tables[bank][indices[bank]];
                    if e.u == 0 {
                        e.tag = tags[bank];
                        e.ctr = if taken { 0 } else { -1 };
                        e.u = 1;
                        allocated = true;
                        break;
                    }
                }
                if !allocated {
                    let banks = self.tables[start_bank..].iter_mut();
                    for (table, &index) in banks.zip(&indices[start_bank..num_banks]) {
                        table[index].u = table[index].u.saturating_sub(1);
                    }
                }
            }
        }
    }

    /// Incrementally updates CSRs for a new speculative branch outcome.
    /// Must be called BEFORE the caller's `ghr.push()`.
    #[inline]
    pub fn speculate(&mut self, taken: bool, ghr: &Ghr) {
        self.geo_banks.update_csrs(taken, ghr);
    }

    /// Recomputes speculative CSRs from a recorded GHR after a squash.
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
        let prediction = tage.predict(0x8000_1000);
        // Default counters are 0, which is >= 0 -> taken.
        assert!(prediction.taken());
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
        let saved = tage.predict(pc);

        // Diverge.
        for _ in 0..30 {
            tage.speculate(true, &ghr);
            ghr.push(true);
        }

        // Repair.
        ghr = snapshot;
        tage.repair(&ghr);
        let restored = tage.predict(pc);
        assert_eq!(saved.taken(), restored.taken());
    }

    #[test]
    fn a_branch_trains_the_entries_its_prediction_read() {
        let mut tage = TageCore::new(&test_config());
        let mut ghr = Ghr::with_len(tage.max_history());
        for i in 0u64..40 {
            tage.speculate(i % 3 == 0, &ghr);
            ghr.push(i % 3 == 0);
        }
        let at_prediction = ghr;
        let pc = 0x8000_2040u64;

        for _ in 0..30 {
            ghr = at_prediction;
            tage.repair(&ghr);
            let prediction = tage.predict(pc);
            for _ in 0..8 {
                tage.speculate(true, &ghr);
                ghr.push(true);
            }
            tage.update(false, &prediction);
        }
        tage.repair(&at_prediction);

        assert!(!tage.predict(pc).taken(), "trained under the history it predicted with");
    }

    #[test]
    fn test_update_trains_predictor() {
        let config = test_config();
        let mut tage = TageCore::new(&config);
        let pc = 0x8000_1000u64;

        // Train not-taken heavily.
        for _ in 0..50 {
            let prediction = tage.predict(pc);
            tage.update(false, &prediction);
        }

        assert!(!tage.predict(pc).taken(), "Should predict not-taken after heavy training");
    }
}
