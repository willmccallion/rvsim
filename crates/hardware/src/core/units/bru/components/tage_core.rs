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

/// The bimodal's starting counter, weakly not taken: `TAGEBase` starts
/// each entry's prediction bit clear and its hysteresis bit set.
const BASE_WEAKLY_NOT_TAKEN: i8 = -1;

/// The bimodal's 2-bit counter range.
const BASE_MIN: i8 = -2;
const BASE_MAX: i8 = 1;

/// Seed of the generator allocation draws from.
const RANDOM_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// Moves `ctr` one step toward `up`, within `[min, max]`.
const fn saturating_step(ctr: i8, up: bool, min: i8, max: i8) -> i8 {
    if up {
        if ctr < max { ctr + 1 } else { ctr }
    } else if ctr > min {
        ctr - 1
    } else {
        ctr
    }
}

/// Tagged banks that share a `USE_ALT_ON_NA` counter when there are
/// several, as TAGE-SC-L groups its tables.
const USE_ALT_BANK_GROUP: usize = 8;

/// `|2 * ctr + 1|`, how far a counter is from its weak middle.
const fn centred_magnitude(ctr: i8) -> u32 {
    (2 * ctr as i32 + 1).unsigned_abs()
}

/// The range of a signed `bits`-wide counter.
const fn signed_range(bits: u32) -> (i8, i8) {
    let max = ((1i32 << (bits - 1)) - 1) as i8;
    (-max - 1, max)
}

/// Branches the path history holds, as gem5's `pathHistBits`.
const PATH_HISTORY_BITS: usize = 16;

/// `TAGEBase::F`: folds the low `size` bits of path history `path` into a
/// `table_bits`-wide value, rotated by `bank` so each table hashes it
/// differently.
const fn fold_path(path: u32, size: usize, bank: usize, table_bits: usize) -> u32 {
    let mask = (1u32 << table_bits) - 1;
    let path = path & ((1u32 << size) - 1);
    let high = rotate_left(path >> table_bits, bank, table_bits);
    rotate_left((path & mask) ^ high, bank, table_bits)
}

/// Rotates the `width`-bit `value` left by `amount` bits.
const fn rotate_left(value: u32, amount: usize, width: usize) -> u32 {
    let mask = (1u32 << width) - 1;
    let shift = amount % width;
    if shift == 0 {
        value & mask
    } else {
        ((value << shift) & mask) | ((value & mask) >> (width - shift))
    }
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
    /// Bank of the next longest matching entry.
    alt: Option<usize>,
    /// The longest match's prediction, or the base's without one.
    provider_taken: bool,
    /// The next longest match's prediction, or the base's without one.
    alt_taken: bool,
    /// The longest match's counter is weak, as a new entry's is.
    provider_weak: bool,
    /// The `USE_ALT_ON_NA` counter that chose between them.
    use_alt_index: usize,
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
    /// `USE_ALT_ON_NA`: whether a weak (newly allocated) provider defers
    /// to the alternate prediction; non-negative defers.
    use_alt_on_na: Vec<i8>,
    use_alt_bits: u32,
    /// Low bit of each recent branch's `pc >> 2`, youngest in bit 0.
    path_history: u16,
    /// Counts updates; the useful bits age each time it passes a multiple
    /// of `reset_interval` (gem5's `tCounter`).
    update_count: u64,
    reset_interval: u64,
    /// Chooses among the tables an allocation may start at.
    random: u64,
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
            base: vec![BASE_WEAKLY_NOT_TAKEN; config.table_size],
            geo_banks,
            tables,
            use_alt_on_na: vec![0; config.use_alt_counters.max(1)],
            use_alt_bits: config.use_alt_bits,
            path_history: 0,
            // Half a period in, as gem5's initialTCounterValue (2^17) is for
            // its 2^18-update period.
            update_count: u64::from(config.reset_interval / 2),
            reset_interval: u64::from(config.reset_interval.max(1)),
            random: RANDOM_SEED,
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
            indices[bank] = self.index(pc, bank);
            tags[bank] = self.tag(pc, bank);
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
        let base_saturated = base_ctr == BASE_MIN || base_ctr == BASE_MAX;
        let alt_confident = alt.map_or(base_saturated, |_| centred_magnitude(alt_ctr) > 1);
        let use_alt_index = self.use_alt_index(provider, alt_confident);
        let pred_ctr = if provider_weak && self.use_alt_on_na[use_alt_index] >= 0 {
            alt_ctr
        } else {
            provider_ctr
        };
        let conf = if provider.is_some() {
            TageConfLevel::from_tagged_ctr(provider_ctr)
        } else {
            TageConfLevel::from_bimodal(base_saturated)
        };
        let meta = TageScMeta {
            conf,
            provider_bank: provider.map_or(0, |b| b + 1),
            alt_bank_present: alt.is_some(),
            pred_taken: pred_ctr >= 0,
            provider_disagrees_with_alt: (provider_ctr >= 0) != (alt_ctr >= 0),
        };
        TagePrediction {
            indices,
            tags,
            base_index,
            provider,
            alt,
            provider_taken: provider_ctr >= 0,
            alt_taken: alt_ctr >= 0,
            provider_weak,
            use_alt_index,
            meta,
        }
    }

    /// The `USE_ALT_ON_NA` counter for a prediction: `TAGEBase`'s one, or
    /// `TAGE_SC_L_TAGE::getUseAltIdx`'s pick by the provider's group of
    /// banks and whether the alternate is confident.
    fn use_alt_index(&self, provider: Option<usize>, alt_confident: bool) -> usize {
        let counters = self.use_alt_on_na.len();
        if counters == 1 {
            return 0;
        }
        let group = provider.map_or(0, |bank| bank / USE_ALT_BANK_GROUP);
        ((group << 1) + usize::from(alt_confident)) % (counters - 1)
    }

    /// Trains the entries `prediction` read with the branch's outcome, and
    /// allocates a longer-history entry when it was wrong, as
    /// `TAGEBase::condBranchUpdate` does.
    pub fn update(&mut self, taken: bool, prediction: &TagePrediction) {
        let num_banks = self.tables.len();
        let mut allocate = prediction.taken() != taken
            && prediction.provider.is_none_or(|bank| bank + 1 < num_banks);
        if prediction.provider.is_some() && prediction.provider_weak {
            // A new entry that was right needs no longer one.
            if prediction.provider_taken == taken {
                allocate = false;
            }
            if prediction.provider_taken != prediction.alt_taken {
                let alt_right = prediction.alt_taken == taken;
                let (min, max) = signed_range(self.use_alt_bits);
                let ctr = &mut self.use_alt_on_na[prediction.use_alt_index];
                *ctr = saturating_step(*ctr, alt_right, min, max);
            }
        }
        let choice = self.next_random();
        if allocate {
            self.allocate(taken, prediction, choice);
        }
        self.age_useful_bits();
        self.train(taken, prediction);
    }

    /// Takes one entry in a table longer than the provider's for the
    /// branch, as `TAGEBase::handleAllocAndUReset` does: it starts at one
    /// of the next three tables, chosen at random so entries do not
    /// ping-pong, and when none from there is free it frees that one.
    fn allocate(&mut self, taken: bool, prediction: &TagePrediction, choice: u64) {
        let num_banks = self.tables.len();
        let TagePrediction { indices, tags, .. } = *prediction;
        let first = prediction.provider.map_or(0, |bank| bank + 1);
        let free = |table: &Vec<TageEntry>, bank: usize| table[indices[bank]].u == 0;
        let any_free = (first..num_banks).any(|bank| free(&self.tables[bank], bank));
        let skips = choice & ((1 << (num_banks - first - 1)) - 1);
        let start = first + usize::from(skips & 1 != 0) + usize::from(skips & 0b11 == 0b11);
        if !any_free {
            self.tables[start][indices[start]].u = 0;
        }
        if let Some(bank) = (start..num_banks).find(|&bank| free(&self.tables[bank], bank)) {
            let entry = &mut self.tables[bank][indices[bank]];
            entry.tag = tags[bank];
            entry.ctr = if taken { 0 } else { -1 };
        }
    }

    /// Halves every useful bit once per `reset_interval` updates.
    fn age_useful_bits(&mut self) {
        self.update_count += 1;
        if self.update_count.is_multiple_of(self.reset_interval) {
            for entry in self.tables.iter_mut().flatten() {
                entry.u >>= 1;
            }
        }
    }

    /// Trains the provider, and the alternate too while the provider has
    /// not proved useful, as `TAGEBase::handleTAGEUpdate` does.
    fn train(&mut self, taken: bool, prediction: &TagePrediction) {
        let TagePrediction { indices, base_index, provider, alt, .. } = *prediction;
        let Some(bank) = provider else {
            self.train_base(base_index, taken);
            return;
        };
        let entry = &mut self.tables[bank][indices[bank]];
        entry.ctr = saturating_step(entry.ctr, taken, -4, 3);
        if entry.u == 0 {
            match alt {
                Some(alt) => {
                    let alt_entry = &mut self.tables[alt][indices[alt]];
                    alt_entry.ctr = saturating_step(alt_entry.ctr, taken, -4, 3);
                }
                None => self.train_base(base_index, taken),
            }
        }
        if prediction.taken() != prediction.alt_taken {
            let entry = &mut self.tables[bank][indices[bank]];
            entry.u = if prediction.taken() == taken {
                (entry.u + 1).min(3)
            } else {
                entry.u.saturating_sub(1)
            };
        }
    }

    fn train_base(&mut self, index: usize, taken: bool) {
        self.base[index] = saturating_step(self.base[index], taken, BASE_MIN, BASE_MAX);
    }

    /// The next value of a xorshift generator: gem5 draws from a Mersenne
    /// twister, and a fixed seed keeps runs reproducible.
    const fn next_random(&mut self) -> u64 {
        let mut x = self.random;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.random = x;
        x
    }

    /// The index of `pc` in tagged `bank` (0-based), as `TAGEBase::gindex`
    /// hashes it: the PC, a shifted copy of it, the folded global history
    /// and the folded path history.
    fn index(&self, pc: u64, bank: usize) -> usize {
        let table_bits = self.geo_banks.table_bits();
        let gem5_bank = bank + 1;
        let shifted_pc = (pc >> 2) as u32;
        let (index_fold, _, _) = self.geo_banks.folds(bank);
        let path_bits = self.geo_banks.hist_length(bank).min(PATH_HISTORY_BITS);
        let hash = shifted_pc
            ^ (shifted_pc >> (table_bits.abs_diff(gem5_bank) + 1))
            ^ index_fold as u32
            ^ fold_path(u32::from(self.path_history), path_bits, gem5_bank, table_bits);
        hash as usize & self.geo_banks.table_mask()
    }

    /// The tag of `pc` in tagged `bank` (0-based), as `TAGEBase::gtag`
    /// forms it from the PC and two folds of the global history.
    const fn tag(&self, pc: u64, bank: usize) -> u16 {
        let width = self.geo_banks.tag_width(bank);
        let (_, tag_fold, short_tag_fold) = self.geo_banks.folds(bank);
        let tag = (pc >> 2) ^ tag_fold ^ (short_tag_fold << 1);
        (tag & ((1 << width) - 1)) as u16
    }

    /// Shifts a branch at `pc` with direction `taken` into the speculative
    /// global and path histories. Must be called BEFORE the caller's
    /// `ghr.push()`.
    #[inline]
    pub fn speculate(&mut self, pc: u64, taken: bool, ghr: &Ghr) {
        self.geo_banks.update_csrs(taken, ghr);
        self.path_history = (self.path_history << 1) | ((pc >> 2) & 1) as u16;
    }

    /// The speculative path history, for a branch to restore on a squash.
    #[must_use]
    pub const fn path_history(&self) -> u16 {
        self.path_history
    }

    /// Restores the speculative histories a squash returns to.
    pub fn repair(&mut self, ghr: &Ghr, path_history: u16) {
        self.geo_banks.recompute_all(ghr);
        self.path_history = path_history;
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
    fn a_branch_never_seen_is_predicted_not_taken() {
        let tage = TageCore::new(&test_config());

        // Its tags are not 0, which every empty tagged entry holds.
        let prediction = tage.predict(0x8000_1004);

        assert!(!prediction.taken());
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
            tage.speculate(0x8000_0000, taken, &ghr);
            ghr.push(taken);
        }

        let snapshot = ghr;
        let saved = tage.predict(pc);

        // Diverge.
        for _ in 0..30 {
            tage.speculate(0x8000_0000, true, &ghr);
            ghr.push(true);
        }

        // Repair.
        ghr = snapshot;
        tage.repair(&ghr, 0);
        let restored = tage.predict(pc);
        assert_eq!(saved.taken(), restored.taken());
    }

    #[test]
    fn a_branch_trains_the_entries_its_prediction_read() {
        let mut tage = TageCore::new(&test_config());
        let mut ghr = Ghr::with_len(tage.max_history());
        for i in 0u64..40 {
            tage.speculate(0x8000_0000 + 4 * i, i % 3 == 0, &ghr);
            ghr.push(i % 3 == 0);
        }
        let at_prediction = ghr;
        let pc = 0x8000_2040u64;

        for _ in 0..30 {
            ghr = at_prediction;
            tage.repair(&ghr, 0);
            let prediction = tage.predict(pc);
            for _ in 0..8 {
                tage.speculate(0x8000_0000, true, &ghr);
                ghr.push(true);
            }
            tage.update(false, &prediction);
        }
        tage.repair(&at_prediction, 0);

        assert!(!tage.predict(pc).taken(), "trained under the history it predicted with");
    }

    #[test]
    fn the_path_fold_matches_tagebase_f() {
        assert_eq!(fold_path(0xABCD, 16, 1, 11), 0x7CE);
        assert_eq!(fold_path(0xABCD, 16, 2, 11), 0x665);
        assert_eq!(fold_path(0xABCD, 5, 1, 11), fold_path(0xD, 5, 1, 11));
    }

    #[test]
    fn the_path_history_separates_branches_with_the_same_global_history() {
        let mut tage = TageCore::new(&test_config());
        let ghr = Ghr::with_len(tage.max_history());
        let pc = 0x8000_2040u64;

        tage.repair(&ghr, 0b1010);
        let one_path = tage.predict(pc);
        tage.repair(&ghr, 0b0101);
        let other_path = tage.predict(pc);

        assert_ne!(one_path.indices, other_path.indices);
    }

    /// A prediction that read entry `bank * 16` in every table, with the
    /// given provider, alternate and predictions.
    fn prediction(
        provider: Option<usize>,
        alt: Option<usize>,
        provider_taken: bool,
        alt_taken: bool,
        taken: bool,
    ) -> TagePrediction {
        let mut indices = [0usize; MAX_BANKS];
        let mut tags = [0u16; MAX_BANKS];
        for bank in 0..4 {
            indices[bank] = bank * 16;
            tags[bank] = 0x55 + bank as u16;
        }
        TagePrediction {
            indices,
            tags,
            base_index: 0,
            provider,
            alt,
            provider_taken,
            alt_taken,
            provider_weak: true,
            use_alt_index: 0,
            meta: TageScMeta {
                conf: TageConfLevel::None,
                provider_bank: provider.map_or(0, |bank| bank + 1),
                alt_bank_present: alt.is_some(),
                pred_taken: taken,
                provider_disagrees_with_alt: provider_taken != alt_taken,
            },
        }
    }

    fn allocated_banks(tage: &TageCore, prediction: &TagePrediction) -> Vec<usize> {
        (0..4)
            .filter(|&bank| {
                tage.tables[bank][prediction.indices[bank]].tag == prediction.tags[bank]
            })
            .collect()
    }

    #[test]
    fn a_mispredict_with_no_free_entry_frees_one_of_the_next_three_tables() {
        let mut tage = TageCore::new(&test_config());
        let wrong = prediction(None, None, true, true, true);
        for bank in 0..4 {
            tage.tables[bank][wrong.indices[bank]].u = 3;
        }

        tage.update(false, &wrong);

        let taken = allocated_banks(&tage, &wrong);
        assert_eq!(taken.len(), 1, "one entry allocated");
        assert!(taken[0] <= 2, "among the next three tables, got {}", taken[0]);
    }

    #[test]
    fn a_weak_new_entry_that_was_right_allocates_nothing() {
        let mut tage = TageCore::new(&test_config());
        // The alternate overrode a new provider that was right.
        let overridden = prediction(Some(0), None, false, true, true);

        tage.update(false, &overridden);

        assert!(allocated_banks(&tage, &overridden).is_empty());
    }

    #[test]
    fn the_alternate_trains_while_the_provider_has_not_proved_useful() {
        let mut tage = TageCore::new(&test_config());
        let p = prediction(Some(1), Some(0), true, true, true);

        tage.update(false, &p);

        assert_eq!(tage.tables[0][p.indices[0]].ctr, -1, "the alternate moved toward not taken");
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

    #[test]
    fn several_use_alt_counters_split_by_bank_group_and_alternate_confidence() {
        let tage = TageCore::new(&TageConfig {
            num_banks: 12,
            history_lengths: (1..=12).map(|i| i * 10).collect(),
            tag_widths: vec![10; 12],
            use_alt_counters: 16,
            use_alt_bits: 5,
            ..TageConfig::default()
        });

        let picks = [
            tage.use_alt_index(Some(0), false),
            tage.use_alt_index(Some(7), true),
            tage.use_alt_index(Some(8), false),
            tage.use_alt_index(Some(11), true),
        ];

        assert_eq!(picks, [0, 1, 2, 3]);
    }

    #[test]
    fn one_use_alt_counter_serves_every_prediction() {
        let tage = TageCore::new(&test_config());

        assert_eq!(tage.use_alt_index(Some(3), true), 0);
    }
}
