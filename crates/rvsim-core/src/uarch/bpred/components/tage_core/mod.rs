//! Core TAGE direction predictor logic shared by standalone TAGE and SC-L-TAGE.
//!
//! Owns the bimodal base table, tagged banks (`GeoBankSet`), and `USE_ALT_ON_NA`
//! counter. Does NOT own a GHR, BTB, RAS, loop predictor, or SC — callers
//! compose those on top.

use super::sc_types::{TageConfLevel, TageScMeta};
use super::tage_history::{HistoryBranch, HistoryCheckpoint, TageHistories};
use super::tagged_bank::MAX_BANKS;
use crate::config::{TageAllocation, TageBanking, TageConfig, TageHashing, TageUpdate};
use crate::uarch::bpred::components::xorshift64;

/// An entry in a TAGE tagged bank.
#[derive(Clone, Copy, Debug, Default)]
struct TageEntry {
    tag: u16,
    ctr: i8,
    u: u8,
}

/// The 3-bit tagged counter range.
const TAGGED_MIN: i8 = -4;
const TAGGED_MAX: i8 = 3;

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

/// `TAGEBase::F`: folds the low `size` bits of path history `path` into a
/// `table_bits`-wide value, rotated by `bank` so each table hashes it
/// differently. Banks at or past `table_bits` are not rotated, and the bits
/// above the table width are rotated unmasked, as gem5 does for a path
/// wider than twice the table width.
const fn fold_path(path: u32, size: usize, bank: usize, table_bits: usize) -> u32 {
    let mask = (1u64 << table_bits) - 1;
    let path = path as u64 & ((1u64 << size) - 1);
    let high = rotate_by_bank(path >> table_bits, bank, table_bits);
    rotate_by_bank((path & mask) ^ high, bank, table_bits) as u32
}

/// `F`'s rotation of `value` by `bank` within `table_bits`.
const fn rotate_by_bank(value: u64, bank: usize, table_bits: usize) -> u64 {
    if bank < table_bits {
        ((value << bank) & ((1 << table_bits) - 1)) + (value >> (table_bits - bank))
    } else {
        value
    }
}

/// `TAGEBase`'s bimodal: a prediction bit per entry and a hysteresis bit
/// shared by `2^share_log` neighbouring entries, which together act as a
/// 2-bit counter. Every entry starts weakly not taken.
#[derive(Debug)]
struct Bimodal {
    prediction: Vec<bool>,
    hysteresis: Vec<bool>,
    share_log: u32,
}

impl Bimodal {
    fn new(entries: usize, share_log: u32) -> Self {
        Self {
            prediction: vec![false; entries],
            hysteresis: vec![true; entries >> share_log],
            share_log,
        }
    }

    const fn mask(&self) -> usize {
        self.prediction.len() - 1
    }

    /// The entry's prediction and shared hysteresis as a counter from -2
    /// (strongly not taken) to 1 (strongly taken).
    fn counter(&self, index: usize) -> i8 {
        let state = (u8::from(self.prediction[index]) << 1)
            | u8::from(self.hysteresis[index >> self.share_log]);
        state as i8 - 2
    }

    const fn saturated(counter: i8) -> bool {
        counter == -2 || counter == 1
    }

    /// `TAGEBase::baseUpdate`.
    fn train(&mut self, index: usize, taken: bool) {
        let state = self.counter(index) + 2;
        let state = if taken { (state + 1).min(3) } else { (state - 1).max(0) };
        self.prediction[index] = state >> 1 != 0;
        self.hysteresis[index >> self.share_log] = state & 1 != 0;
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
    bimodal: Bimodal,
    histories: TageHistories,
    table_bits: usize,
    hist_lengths: Vec<usize>,
    tag_widths: Vec<usize>,
    path_history_bits: usize,
    /// The entry arrays: one per bank, or TAGE-SC-L's short- and long-tag
    /// arrays that its banks share.
    storage: Vec<Vec<TageEntry>>,
    /// Which array each bank's entries are in.
    bank_storage: Vec<usize>,
    num_banks: usize,
    banking: Option<TageBanking>,
    hashing: TageHashing,
    /// `USE_ALT_ON_NA`: whether a weak (newly allocated) provider defers
    /// to the alternate prediction; non-negative defers.
    use_alt_on_na: Vec<i8>,
    use_alt_bits: u32,
    /// gem5's `tCounter`: counts updates under `TAGEBase` allocation, or
    /// allocations that found no free entry less those that did under
    /// CBP-5; the useful bits age as it reaches `reset_interval`.
    useful_reset_counter: i64,
    reset_interval: i64,
    useful_max: u8,
    max_allocations: usize,
    allocation: TageAllocation,
    update: TageUpdate,
    /// Chooses among the tables an allocation may start at.
    random: u64,
}

impl TageCore {
    /// Creates a new TAGE core from config.
    ///
    /// # Panics
    ///
    /// Panics if `table_size` is not a power of two, or if `history_lengths`
    /// and `tag_widths` have different lengths or more than `MAX_BANKS`.
    pub fn new(config: &TageConfig) -> Self {
        assert!(config.table_size.is_power_of_two(), "TAGE table size must be power of 2");

        let num_banks = config.num_banks;
        let hist_lengths = &config.history_lengths;
        let tag_widths = &config.tag_widths;

        assert_eq!(hist_lengths.len(), num_banks);
        assert_eq!(tag_widths.len(), num_banks);
        assert!(num_banks <= MAX_BANKS, "TAGE: {num_banks} banks exceeds {MAX_BANKS}");

        let table_bits = config.table_size.trailing_zeros() as usize;

        let (storage, bank_storage) = Self::storage(config);

        let histories = TageHistories::new(
            hist_lengths,
            tag_widths,
            table_bits,
            config.path_history_bits,
            config.history,
        );

        Self {
            bimodal: Bimodal::new(config.bimodal_entries(), config.bimodal_hysteresis_share_log),
            histories,
            table_bits,
            hist_lengths: hist_lengths.clone(),
            tag_widths: tag_widths.clone(),
            path_history_bits: config.path_history_bits as usize,
            storage,
            bank_storage,
            num_banks,
            banking: config.banking.clone(),
            hashing: config.hashing,
            use_alt_on_na: vec![0; config.use_alt_counters.max(1)],
            use_alt_bits: config.use_alt_bits,
            // Half a period in, as gem5's initialTCounterValue is.
            useful_reset_counter: i64::from(config.reset_interval / 2),
            reset_interval: i64::from(config.reset_interval.max(1)),
            useful_max: ((1u32 << config.useful_bits) - 1) as u8,
            max_allocations: config.max_allocations,
            allocation: config.allocation,
            update: config.update,
            random: RANDOM_SEED,
        }
    }

    /// The entry arrays and the array each bank is in: a table per bank, or
    /// TAGE-SC-L's short- and long-tag arrays.
    fn storage(config: &TageConfig) -> (Vec<Vec<TageEntry>>, Vec<usize>) {
        let table = |slices: usize| vec![TageEntry::default(); slices * config.table_size];
        let banks = 0..config.num_banks;
        config.banking.as_ref().map_or_else(
            || (banks.clone().map(|_| table(1)).collect(), banks.clone().collect()),
            |banking| {
                let short_or_long =
                    banks.clone().map(|bank| usize::from(bank >= banking.first_long_bank));
                (
                    vec![table(banking.short_factor), table(banking.long_factor)],
                    short_or_long.collect(),
                )
            },
        )
    }

    fn enabled(&self, bank: usize) -> bool {
        self.banking.as_ref().is_none_or(|banking| banking.enabled[bank])
    }

    const fn table_mask(&self) -> usize {
        (1 << self.table_bits) - 1
    }

    /// Predicts the branch at `pc` from the speculative history, recording
    /// the entries it read. `O(num_banks)`.
    pub fn predict(&self, pc: u64) -> TagePrediction {
        let (indices, tags) = self.indices_and_tags(pc);
        let base_index = self.bimodal_index(pc);

        let mut matching = (0..self.num_banks).rev().filter(|&b| {
            self.enabled(b) && self.storage[self.bank_storage[b]][indices[b]].tag == tags[b]
        });
        let provider = matching.next();
        let alt = matching.next();

        let base_ctr = self.bimodal.counter(base_index);
        let ctr_of = |bank: Option<usize>| {
            bank.map_or(base_ctr, |b| self.storage[self.bank_storage[b]][indices[b]].ctr)
        };
        let (provider_ctr, alt_ctr) = (ctr_of(provider), ctr_of(alt));
        let provider_weak = provider.is_some() && (provider_ctr == 0 || provider_ctr == -1);
        let base_saturated = Bimodal::saturated(base_ctr);
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
    /// allocates longer-history entries when it was wrong, as
    /// `TAGEBase::condBranchUpdate` does. `final_taken` is the prediction
    /// fetch followed, which the CBP-5 allocation also weighs.
    pub fn update(&mut self, taken: bool, prediction: &TagePrediction, final_taken: bool) {
        let num_banks = self.num_banks;
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
        match self.allocation {
            TageAllocation::TageBase => {
                let choice = self.next_random();
                if allocate {
                    self.allocate_after_provider(taken, prediction, choice);
                }
                self.age_useful_bits_periodically();
            }
            TageAllocation::Cbp5 => {
                if allocate && final_taken == taken && self.next_random() & 31 != 0 {
                    allocate = false;
                }
                if allocate {
                    self.allocate_in_pairs(taken, prediction);
                }
            }
        }
        match self.update {
            TageUpdate::TageBase => self.train(taken, prediction),
            TageUpdate::Cbp5 => self.train_cbp5(taken, prediction),
        }
    }

    /// Takes free entries in tables longer than the provider's, as
    /// `TAGEBase::handleAllocAndUReset` does: it starts at one of the next
    /// three tables, chosen at random so entries do not ping-pong, and when
    /// none from there is free it frees that one.
    fn allocate_after_provider(&mut self, taken: bool, prediction: &TagePrediction, choice: u64) {
        let num_banks = self.num_banks;
        let TagePrediction { indices, tags, .. } = *prediction;
        let first = prediction.provider.map_or(0, |bank| bank + 1);
        let free = |bank: usize| {
            self.enabled(bank) && self.storage[self.bank_storage[bank]][indices[bank]].u == 0
        };
        let any_free = (first..num_banks).any(free);
        let skips = choice & ((1 << (num_banks - first - 1)) - 1);
        let start = first + usize::from(skips & 1 != 0) + usize::from(skips & 0b11 == 0b11);
        if !any_free {
            self.storage[self.bank_storage[start]][indices[start]].u = 0;
        }
        let mut allocated = 0;
        for bank in start..num_banks {
            if allocated == self.max_allocations {
                break;
            }
            if !self.enabled(bank) {
                continue;
            }
            let entry = &mut self.storage[self.bank_storage[bank]][indices[bank]];
            if entry.u == 0 {
                entry.tag = tags[bank];
                entry.ctr = if taken { 0 } else { -1 };
                allocated += 1;
            }
        }
    }

    /// Halves every useful counter once per `reset_interval` updates.
    fn age_useful_bits_periodically(&mut self) {
        self.useful_reset_counter += 1;
        if self.useful_reset_counter % self.reset_interval == 0 {
            self.halve_useful_bits();
        }
    }

    fn halve_useful_bits(&mut self) {
        for entry in self.storage.iter_mut().flatten() {
            entry.u >>= 1;
        }
    }

    /// CBP-5's allocation (`TAGE_SC_L_TAGE_64KB::handleAllocAndUReset`):
    /// walks the tables above the provider two at a time from a start one
    /// or (one time in four) two pairs up, taking an unuseful entry that is
    /// not strongly biased and skipping the next pair after each, and
    /// decaying the strong ones it passes. Useful entries in the way count
    /// against it; once they outweigh the allocations by `reset_interval`,
    /// every useful counter halves.
    fn allocate_in_pairs(&mut self, taken: bool, prediction: &TagePrediction) {
        let num_banks = self.num_banks;
        let TagePrediction { indices, tags, .. } = *prediction;
        let provider = prediction.provider.map_or(0, |bank| bank + 1);
        let pairs_up = if self.next_random() & 127 < 32 { 2 } else { 1 };
        let mut pair = ((provider + 2 * pairs_up - 1) & !1) ^ (self.next_random() & 1) as usize;
        let (mut penalty, mut allocated) = (0i64, 0usize);
        while pair < num_banks && allocated < self.max_allocations {
            for bank in [pair, pair ^ 1] {
                if bank >= num_banks || !self.enabled(bank) {
                    continue;
                }
                let entry = &mut self.storage[self.bank_storage[bank]][indices[bank]];
                if entry.u != 0 {
                    penalty += 1;
                } else if centred_magnitude(entry.ctr) <= 3 {
                    entry.tag = tags[bank];
                    entry.ctr = if taken { 0 } else { -1 };
                    allocated += 1;
                    pair += 2;
                    break;
                } else if entry.ctr > 0 {
                    entry.ctr -= 1;
                } else {
                    entry.ctr += 1;
                }
            }
            pair += 2;
        }
        self.useful_reset_counter =
            (self.useful_reset_counter + penalty - 2 * allocated as i64).max(0);
        if self.useful_reset_counter >= self.reset_interval {
            self.halve_useful_bits();
            self.useful_reset_counter = 0;
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
        let entry = &mut self.storage[self.bank_storage[bank]][indices[bank]];
        entry.ctr = saturating_step(entry.ctr, taken, TAGGED_MIN, TAGGED_MAX);
        if entry.u == 0 {
            match alt {
                Some(alt) => {
                    let alt_entry = &mut self.storage[self.bank_storage[alt]][indices[alt]];
                    alt_entry.ctr = saturating_step(alt_entry.ctr, taken, TAGGED_MIN, TAGGED_MAX);
                }
                None => self.train_base(base_index, taken),
            }
        }
        if prediction.taken() != prediction.alt_taken {
            let useful_max = self.useful_max;
            let entry = &mut self.storage[self.bank_storage[bank]][indices[bank]];
            entry.u = if prediction.taken() == taken {
                (entry.u + 1).min(useful_max)
            } else {
                entry.u.saturating_sub(1)
            };
        }
    }

    /// CBP-5's training (`TAGE_SC_L_TAGE_64KB::handleTAGEUpdate`): the
    /// alternate learns only when a weak provider is wrong; a provider
    /// that turns weak, or that was right beside a saturated right
    /// alternate, loses its usefulness, which it gains by being right
    /// where the alternate was wrong.
    fn train_cbp5(&mut self, taken: bool, prediction: &TagePrediction) {
        let TagePrediction {
            indices, base_index, provider, alt, provider_taken, alt_taken, ..
        } = *prediction;
        let Some(bank) = provider else {
            self.train_base(base_index, taken);
            return;
        };
        let weak = centred_magnitude(self.storage[self.bank_storage[bank]][indices[bank]].ctr) == 1;
        if weak && provider_taken != taken {
            match alt {
                Some(alt) => {
                    let alt_entry = &mut self.storage[self.bank_storage[alt]][indices[alt]];
                    alt_entry.ctr = saturating_step(alt_entry.ctr, taken, TAGGED_MIN, TAGGED_MAX);
                }
                None => self.train_base(base_index, taken),
            }
        }
        let alt_saturated = alt.is_some_and(|alt| {
            centred_magnitude(self.storage[self.bank_storage[alt]][indices[alt]].ctr) == 7
        });
        let useful_max = self.useful_max;
        let entry = &mut self.storage[self.bank_storage[bank]][indices[bank]];
        entry.ctr = saturating_step(entry.ctr, taken, TAGGED_MIN, TAGGED_MAX);
        if centred_magnitude(entry.ctr) == 1 {
            entry.u = 0;
        }
        if alt_taken == taken && alt_saturated && entry.u == 1 && provider_taken == taken {
            entry.u = 0;
        }
        if provider_taken != alt_taken && provider_taken == taken && entry.u < useful_max {
            entry.u += 1;
        }
    }

    fn train_base(&mut self, index: usize, taken: bool) {
        self.bimodal.train(index, taken);
    }

    /// The next value of a xorshift generator: gem5 draws from a Mersenne
    /// twister, and a fixed seed keeps runs reproducible.
    const fn next_random(&mut self) -> u64 {
        xorshift64(&mut self.random)
    }

    /// The PC as the table and tag hashes take it: shifted past its low
    /// bits by `TAGEBase`, whole by TAGE-SC-L.
    const fn hashed_pc(&self, pc: u64) -> u64 {
        match self.hashing {
            TageHashing::TageBase => pc >> 2,
            TageHashing::TageScL => pc,
        }
    }

    /// `bindex`: `TAGEBase`'s shifted PC, or TAGE-SC-L's `pc ^ (pc >> 2)`.
    const fn bimodal_index(&self, pc: u64) -> usize {
        let hash = match self.hashing {
            TageHashing::TageBase => pc >> 2,
            TageHashing::TageScL => pc ^ (pc >> 2),
        };
        hash as usize & self.bimodal.mask()
    }

    /// Each bank's index and tag for `pc`, as `calculateIndicesAndTags`
    /// computes them. With banking (`TAGE_SC_L_TAGE`'s), the second bank of
    /// each pair takes the first's tag and its index XOR that tag, and each
    /// enabled bank's index moves to its slice of the shared array.
    fn indices_and_tags(&self, pc: u64) -> ([usize; MAX_BANKS], [u16; MAX_BANKS]) {
        let mut indices = [0usize; MAX_BANKS];
        let mut tags = [0u16; MAX_BANKS];
        let Some(banking) = &self.banking else {
            for bank in 0..self.num_banks {
                indices[bank] = self.index(pc, bank);
                tags[bank] = self.tag(pc, bank);
            }
            return (indices, tags);
        };
        for first in (0..self.num_banks).step_by(2) {
            indices[first] = self.index(pc, first);
            tags[first] = self.tag(pc, first);
            if first + 1 < self.num_banks {
                tags[first + 1] = tags[first];
                indices[first + 1] =
                    indices[first] ^ (usize::from(tags[first]) & self.table_mask());
            }
        }
        let long = banking.first_long_bank..self.num_banks;
        self.place_in_slices(pc, long, banking.long_factor, &mut indices);
        self.place_in_slices(pc, 0..banking.first_long_bank, banking.short_factor, &mut indices);
        (indices, tags)
    }

    /// Moves each enabled bank in `banks` to consecutive slices of an array
    /// of `slices`, starting from one hashed from the PC and the path.
    fn place_in_slices(
        &self,
        pc: u64,
        banks: std::ops::Range<usize>,
        slices: usize,
        indices: &mut [usize; MAX_BANKS],
    ) {
        let path_bits = self.hist_lengths[banks.start].min(64) as u32;
        let path = u64::from(self.histories.path()) & (u64::MAX >> (64 - path_bits.max(1)));
        let mut slice = ((pc ^ path) % slices as u64) as usize;
        for bank in banks {
            if self.enabled(bank) {
                indices[bank] += slice << self.table_bits;
                slice = (slice + 1) % slices;
            }
        }
    }

    /// The index of `pc` in tagged `bank` (0-based), as `TAGEBase::gindex`
    /// hashes it: the PC, a shifted copy of it, the folded global history
    /// and the folded path history.
    fn index(&self, pc: u64, bank: usize) -> usize {
        let table_bits = self.table_bits;
        let gem5_bank = bank + 1;
        let shifted_pc = self.hashed_pc(pc) as u32;
        let [index_fold, _, _] = self.histories.folds(bank);
        let path_bits = self.hist_lengths[bank].min(self.path_history_bits);
        let hash = shifted_pc
            ^ (shifted_pc >> (table_bits.abs_diff(gem5_bank) + 1))
            ^ index_fold as u32
            ^ fold_path(self.histories.path(), path_bits, gem5_bank, table_bits);
        hash as usize & self.table_mask()
    }

    /// The tag of `pc` in tagged `bank` (0-based), as `TAGEBase::gtag`
    /// forms it from the PC and two folds of the global history.
    fn tag(&self, pc: u64, bank: usize) -> u16 {
        let width = self.tag_widths[bank];
        let [_, tag_fold, short_tag_fold] = self.histories.folds(bank);
        let tag = self.hashed_pc(pc) ^ tag_fold ^ (short_tag_fold << 1);
        (tag & ((1 << width) - 1)) as u16
    }

    /// Shifts the control instruction at `pc` going `taken` into the
    /// speculative histories.
    pub fn speculate(&mut self, pc: u64, taken: bool, branch: HistoryBranch) {
        self.histories.speculate(pc, taken, branch);
    }

    /// The histories before the next instruction's update, which a squash
    /// of that instruction restores.
    #[must_use]
    pub fn checkpoint(&self) -> HistoryCheckpoint {
        self.histories.checkpoint()
    }

    /// Returns the speculative histories to `checkpoint`.
    pub fn restore(&mut self, checkpoint: &HistoryCheckpoint) {
        self.histories.restore(checkpoint);
    }

    /// The speculative path history.
    #[must_use]
    pub const fn path_history(&self) -> u32 {
        self.histories.path()
    }
}

#[cfg(test)]
mod tests;
