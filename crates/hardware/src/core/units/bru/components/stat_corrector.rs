//! Seznec's statistical corrector, as the CBP-5 TAGE-SC-L and gem5's
//! `StatisticalCorrector` build it.
//!
//! Three bias tables and a set of GEHL components each vote a sum of
//! centred counters, scaled by a learnt per-component weight. When the
//! total disagrees with the prediction before it (TAGE's, or the loop
//! predictor's), two choosers keyed on TAGE's confidence and the total's
//! magnitude decide which to follow. The histories the components read
//! advance speculatively with each predicted conditional branch; a
//! prediction keeps the values it read, which restore them on a squash and
//! index the same counters when it trains.

use super::sc_types::{TageConfLevel, TageScMeta};
use crate::config::{GehlConfig, LocalGehlConfig, MAX_LOCAL_HISTORIES, ScConfig};

/// `ctr` one step toward `up`, saturating as a signed `bits`-wide counter.
const fn stepped(ctr: i32, up: bool, bits: u32) -> i32 {
    let max = (1 << (bits - 1)) - 1;
    let min = -(1 << (bits - 1));
    if up {
        if ctr < max { ctr + 1 } else { ctr }
    } else if ctr > min {
        ctr - 1
    } else {
        ctr
    }
}

fn step(ctr: &mut i8, up: bool, bits: u32) {
    *ctr = stepped(i32::from(*ctr), up, bits) as i8;
}

const fn centred(ctr: i8) -> i32 {
    2 * ctr as i32 + 1
}

/// The low `bits` bits set.
const fn low_mask(bits: u32) -> u64 {
    if bits >= 64 { u64::MAX } else { (1 << bits) - 1 }
}

/// `pc ^ (pc >> 2)`, the PC hash the per-PC tables share.
const fn pc_hash(pc: u64) -> u64 {
    pc ^ (pc >> 2)
}

/// The history a GEHL component hashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Global,
    Backward,
    Path,
    Local(usize),
    Imli,
    ImliHistory,
}

/// The corrector's histories as one branch read them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ScHistories {
    global: u64,
    backward: u64,
    path: u64,
    local: [u64; MAX_LOCAL_HISTORIES],
    imli_count: u32,
    imli_history: u64,
}

impl ScHistories {
    const fn of(&self, source: Source) -> u64 {
        match source {
            Source::Global => self.global,
            Source::Backward => self.backward,
            Source::Path => self.path,
            Source::Local(i) => self.local[i],
            Source::Imli => self.imli_count as u64,
            Source::ImliHistory => self.imli_history,
        }
    }
}

/// A GEHL component: one counter table per history length.
#[derive(Debug)]
struct Gehl {
    source: Source,
    lengths: Vec<u32>,
    tables: Vec<Vec<i8>>,
    weights: Vec<i8>,
}

impl Gehl {
    fn new(source: Source, config: &GehlConfig, halve_short_tables: bool, weights: usize) -> Self {
        let count = config.lengths.len();
        let tables = (0..count)
            .map(|table| {
                let halved = halve_short_tables && table + 2 >= count;
                Self::initial_table(1 << (config.log_entries - u32::from(halved)))
            })
            .collect();
        Self {
            source,
            lengths: config.lengths.clone(),
            tables,
            weights: vec![config.weight_init; weights],
        }
    }

    /// gem5's `initGEHLTable`: even entries start at -1, odd ones at 0.
    fn initial_table(entries: usize) -> Vec<i8> {
        (0..entries).map(|j| if j % 2 == 0 && j + 1 < entries { -1 } else { 0 }).collect()
    }

    /// The PC the component hashes: the backward-history component also
    /// hashes the prediction before the corrector.
    fn hashed_pc(&self, pc: u64, before_sc: bool) -> u64 {
        if self.source == Source::Backward { (pc << 1) + u64::from(before_sc) } else { pc }
    }

    /// `StatisticalCorrector::gIndex`.
    fn index(&self, table: usize, pc: u64, history: u64) -> usize {
        let h = history & low_mask(self.lengths[table]);
        let i = table as u32;
        let hash = pc
            ^ h
            ^ (h >> (8 - i))
            ^ (h >> (16 - 2 * i))
            ^ (h >> (24 - 3 * i))
            ^ (h >> (32 - 3 * i))
            ^ (h >> (40 - 4 * i));
        hash as usize & (self.tables[table].len() - 1)
    }

    /// The weighted vote, `StatisticalCorrector::gPredict`.
    fn vote(&self, pc: u64, before_sc: bool, histories: &ScHistories, weight: usize) -> i32 {
        let (pc, history) = (self.hashed_pc(pc, before_sc), histories.of(self.source));
        let sum: i32 = (0..self.tables.len())
            .map(|table| centred(self.tables[table][self.index(table, pc, history)]))
            .sum();
        (1 + i32::from(self.weights[weight] >= 0)) * sum
    }

    /// `StatisticalCorrector::gUpdate`: trains the counters and moves the
    /// weight when this component's vote decided the total's sign.
    fn train(&mut self, prediction: &ScPrediction, taken: bool, weight: usize, bits: &Widths) {
        let pc = self.hashed_pc(prediction.pc, prediction.before_sc);
        let history = prediction.histories.of(self.source);
        let mut sum = 0;
        for table in 0..self.tables.len() {
            let index = self.index(table, pc, history);
            let ctr = &mut self.tables[table][index];
            sum += centred(*ctr);
            step(ctr, taken, bits.counter);
        }
        let weight = &mut self.weights[weight];
        let others = prediction.sum - i32::from(*weight >= 0) * sum;
        if (others + sum >= 0) != (others >= 0) {
            step(weight, (sum >= 0) == taken, bits.weight);
        }
    }

    /// Whether this component's weight raises the threshold.
    fn weight_counts_in_threshold(&self) -> bool {
        self.source != Source::ImliHistory
    }
}

/// Per-branch local histories.
#[derive(Debug)]
struct LocalHistories {
    histories: Vec<u64>,
    index_shift: u32,
    mix_pc: bool,
}

impl LocalHistories {
    fn new(config: &LocalGehlConfig) -> Self {
        Self {
            histories: vec![0; config.histories],
            index_shift: config.index_shift,
            mix_pc: config.mix_pc,
        }
    }

    const fn entry(&self, pc: u64) -> usize {
        (pc ^ (pc >> self.index_shift)) as usize & (self.histories.len() - 1)
    }

    fn get(&self, pc: u64) -> u64 {
        self.histories[self.entry(pc)]
    }

    fn push(&mut self, pc: u64, taken: bool) {
        let entry = self.entry(pc);
        let pushed = (self.histories[entry] << 1) | u64::from(taken);
        self.histories[entry] = if self.mix_pc { pushed ^ (pc & 15) } else { pushed };
    }

    fn restore(&mut self, pc: u64, history: u64) {
        let entry = self.entry(pc);
        self.histories[entry] = history;
    }
}

/// Counter widths the corrector saturates at.
#[derive(Clone, Copy, Debug)]
struct Widths {
    counter: u32,
    weight: u32,
    chooser: u32,
    threshold: u32,
    per_pc_threshold: u32,
}

/// Where a branch's per-PC state lives.
#[derive(Clone, Copy, Debug)]
struct Keys {
    bias: usize,
    bias_sk: usize,
    bias_bank: usize,
    per_pc_threshold: usize,
    weight: usize,
}

/// What a corrector prediction read and decided, carried to commit.
#[derive(Clone, Copy, Debug)]
pub struct ScPrediction {
    pc: u64,
    /// The branch goes backward when taken.
    backward: bool,
    histories: ScHistories,
    tage: TageScMeta,
    before_sc: bool,
    sum: i32,
    threshold: i32,
    taken: bool,
}

impl ScPrediction {
    /// The final prediction.
    #[must_use]
    pub const fn taken(&self) -> bool {
        self.taken
    }
}

/// Seznec's statistical corrector.
#[derive(Debug)]
pub struct StatCorrector {
    bias: Vec<i8>,
    bias_sk: Vec<i8>,
    bias_bank: Vec<i8>,
    bias_weights: Vec<i8>,
    log_bias: u32,
    gehls: Vec<Gehl>,
    /// In eighths.
    threshold: i32,
    per_pc_thresholds: Vec<i32>,
    threshold_weight_step: i32,
    /// `FirstH`: whether to follow the corrector against medium-confidence
    /// TAGE when its sum is small; negative follows it.
    first_chooser: i8,
    /// `SecondH`: the same against high-confidence TAGE.
    second_chooser: i8,
    widths: Widths,
    global: u64,
    backward: u64,
    locals: Vec<LocalHistories>,
    imli_count: u32,
    imli_max: u32,
    /// One history per IMLI count.
    imli_histories: Vec<u64>,
}

impl StatCorrector {
    /// Creates a corrector from a validated config.
    pub fn new(config: &ScConfig) -> Self {
        let weights = 1 << (config.per_pc_threshold_bits / 2);
        let halve = config.halve_short_tables;
        let named = [
            (Source::Global, &config.global),
            (Source::Backward, &config.backward),
            (Source::Path, &config.path),
            (Source::Imli, &config.imli),
            (Source::ImliHistory, &config.imli_history),
        ];
        let locals =
            config.local.iter().enumerate().map(|(i, local)| (Source::Local(i), &local.gehl));
        let gehls = named
            .into_iter()
            .chain(locals)
            .filter(|(_, gehl)| !gehl.lengths.is_empty())
            .map(|(source, gehl)| Gehl::new(source, gehl, halve, weights))
            .collect();
        let (bias, bias_sk, bias_bank) = Self::initial_bias(config.log_bias, config.counter_bits);
        Self {
            bias,
            bias_sk,
            bias_bank,
            bias_weights: vec![config.bias_weight_init; weights],
            log_bias: config.log_bias,
            gehls,
            threshold: config.initial_threshold << 3,
            per_pc_thresholds: vec![
                config.initial_per_pc_threshold;
                1 << config.per_pc_threshold_bits
            ],
            threshold_weight_step: config.threshold_weight_step,
            first_chooser: 0,
            second_chooser: 0,
            widths: Widths {
                counter: config.counter_bits,
                weight: config.weight_bits,
                chooser: config.chooser_bits,
                threshold: config.threshold_bits,
                per_pc_threshold: config.per_pc_threshold_width,
            },
            global: 0,
            backward: 0,
            locals: config.local.iter().map(LocalHistories::new).collect(),
            imli_count: 0,
            imli_max: (1 << config.imli_counter_bits) - 1,
            imli_histories: vec![0; 1 << config.imli_counter_bits],
        }
    }

    /// `StatisticalCorrector::initBias`: the entries for each prediction
    /// before the corrector start agreeing with it, strongly where the
    /// index's confidence bit is clear for `bias` and set for `bias_sk`.
    fn initial_bias(log_bias: u32, counter_bits: u32) -> (Vec<i8>, Vec<i8>, Vec<i8>) {
        let max = ((1 << (counter_bits - 1)) - 1) as i8;
        let min = -max - 1;
        let bias = |j: usize| [min, max, -1, 0][j & 3];
        let bias_sk = |j: usize| [min >> 2, max >> 2, min, max][j & 3];
        let entries = 0..1usize << log_bias;
        (
            entries.clone().map(bias).collect(),
            entries.clone().map(bias_sk).collect(),
            entries.map(bias).collect(),
        )
    }

    /// Corrects `before_sc`, the prediction TAGE (or the loop predictor)
    /// made for the conditional branch at `pc` that goes to `target`.
    /// `path` is TAGE's path history.
    pub fn predict(
        &self,
        pc: u64,
        target: u64,
        path: u64,
        tage: TageScMeta,
        before_sc: bool,
    ) -> ScPrediction {
        let histories = self.histories(pc, path);
        let keys = self.keys(pc, &tage, before_sc);
        let bias_sum = centred(self.bias[keys.bias])
            + centred(self.bias_sk[keys.bias_sk])
            + centred(self.bias_bank[keys.bias_bank]);
        let mut sum = (1 + i32::from(self.bias_weights[keys.weight] >= 0)) * bias_sum;
        for gehl in &self.gehls {
            sum += gehl.vote(pc, before_sc, &histories, keys.weight);
        }
        let threshold = self.threshold(&keys);
        ScPrediction {
            pc,
            backward: target < pc,
            histories,
            tage,
            before_sc,
            sum,
            threshold,
            taken: self.choose(tage.conf, before_sc, sum, threshold),
        }
    }

    /// Follows the corrector's sign where it disagrees, unless TAGE is
    /// confident and the sum small enough that the choosers say not to.
    const fn choose(&self, conf: TageConfLevel, before_sc: bool, sum: i32, threshold: i32) -> bool {
        let sc_taken = sum >= 0;
        if sc_taken == before_sc {
            return before_sc;
        }
        let magnitude = sum.abs();
        let use_sc = match conf {
            TageConfLevel::High if magnitude < threshold / 4 => false,
            TageConfLevel::High if magnitude < threshold / 2 => self.second_chooser < 0,
            TageConfLevel::Medium if magnitude < threshold / 4 => self.first_chooser < 0,
            _ => true,
        };
        if use_sc { sc_taken } else { before_sc }
    }

    fn histories(&self, pc: u64, path: u64) -> ScHistories {
        let mut local = [0; MAX_LOCAL_HISTORIES];
        for (history, table) in local.iter_mut().zip(&self.locals) {
            *history = table.get(pc);
        }
        ScHistories {
            global: self.global,
            backward: self.backward,
            path,
            local,
            imli_count: self.imli_count,
            imli_history: self.imli_histories[self.imli_count as usize],
        }
    }

    fn keys(&self, pc: u64, tage: &TageScMeta, before_sc: bool) -> Keys {
        let low = u64::from(tage.conf == TageConfLevel::Low);
        let high = u64::from(tage.conf == TageConfLevel::High);
        let before = u64::from(before_sc);
        let bias_bit = u64::from(tage.provider_disagrees_with_alt);
        let bias_mask = low_mask(self.log_bias);
        let bias = (((pc_hash(pc) << 1) ^ (low & bias_bit)) << 1) + before;
        let bias_sk = ((((pc ^ (pc >> (self.log_bias - 2))) << 1) ^ high) << 1) + before;
        let bias_bank = before
            + ((((tage.provider_bank + 1) / 4) as u64) << 4)
            + (high << 1)
            + (low << 2)
            + (u64::from(tage.alt_bank_present) << 3)
            + (pc_hash(pc) << 7);
        Keys {
            bias: (bias & bias_mask) as usize,
            bias_sk: (bias_sk & bias_mask) as usize,
            bias_bank: (bias_bank & bias_mask) as usize,
            per_pc_threshold: pc_hash(pc) as usize & (self.per_pc_thresholds.len() - 1),
            weight: pc_hash(pc) as usize & (self.bias_weights.len() - 1),
        }
    }

    /// The global threshold, the branch's own adjustment, and a step per
    /// component whose weight is doubling its vote.
    fn threshold(&self, keys: &Keys) -> i32 {
        let doubled = |weights: &[i8]| i32::from(weights[keys.weight] >= 0);
        let doubled_components: i32 = doubled(&self.bias_weights)
            + self
                .gehls
                .iter()
                .filter(|gehl| gehl.weight_counts_in_threshold())
                .map(|gehl| doubled(&gehl.weights))
                .sum::<i32>();
        (self.threshold >> 3)
            + self.per_pc_thresholds[keys.per_pc_threshold]
            + self.threshold_weight_step * doubled_components
    }

    /// Shifts the histories by a predicted conditional branch's direction.
    pub fn speculate(&mut self, prediction: &ScPrediction, taken: bool) {
        let count = self.imli_count as usize;
        self.imli_histories[count] = (self.imli_histories[count] << 1) | u64::from(taken);
        self.global = (self.global << 1) | u64::from(taken);
        for local in &mut self.locals {
            local.push(prediction.pc, taken);
        }
        if prediction.backward {
            self.imli_count = if taken { (self.imli_count + 1).min(self.imli_max) } else { 0 };
        }
        self.backward = (self.backward << 1) | u64::from(taken && prediction.backward);
    }

    /// Undoes [`Self::speculate`] for a squashed prediction; squashed
    /// predictions are undone youngest first.
    pub fn squash(&mut self, prediction: &ScPrediction) {
        let read = &prediction.histories;
        self.global = read.global;
        self.backward = read.backward;
        self.imli_count = read.imli_count;
        self.imli_histories[read.imli_count as usize] = read.imli_history;
        for (local, &history) in self.locals.iter_mut().zip(&read.local) {
            local.restore(prediction.pc, history);
        }
    }

    /// Trains on a committed branch: `StatisticalCorrector::condBranchUpdate`.
    pub fn update(&mut self, prediction: &ScPrediction, taken: bool) {
        let keys = self.keys(prediction.pc, &prediction.tage, prediction.before_sc);
        let sc_taken = prediction.sum >= 0;
        if sc_taken != prediction.before_sc {
            self.train_choosers(prediction, taken);
        }
        if sc_taken == taken && prediction.sum.abs() >= prediction.threshold {
            return;
        }
        let wrong = sc_taken != taken;
        self.threshold = stepped(self.threshold, wrong, self.widths.threshold);
        let per_pc = &mut self.per_pc_thresholds[keys.per_pc_threshold];
        *per_pc = stepped(*per_pc, wrong, self.widths.per_pc_threshold);
        self.train_bias(&keys, prediction.sum, taken);
        let widths = self.widths;
        for gehl in &mut self.gehls {
            gehl.train(prediction, taken, keys.weight, &widths);
        }
    }

    /// Moves each chooser toward TAGE when TAGE was right in the zone
    /// that chooser decides.
    fn train_choosers(&mut self, prediction: &ScPrediction, taken: bool) {
        let magnitude = prediction.sum.abs();
        let threshold = prediction.threshold;
        let tage_right = prediction.before_sc == taken;
        let bits = self.widths.chooser;
        let second_zone =
            magnitude < threshold && magnitude < threshold / 2 && magnitude >= threshold / 4;
        if prediction.tage.conf == TageConfLevel::High && second_zone {
            step(&mut self.second_chooser, tage_right, bits);
        }
        if prediction.tage.conf == TageConfLevel::Medium && magnitude < threshold / 4 {
            step(&mut self.first_chooser, tage_right, bits);
        }
    }

    fn train_bias(&mut self, keys: &Keys, sum: i32, taken: bool) {
        let bias_sum = centred(self.bias[keys.bias])
            + centred(self.bias_sk[keys.bias_sk])
            + centred(self.bias_bank[keys.bias_bank]);
        let weight = &mut self.bias_weights[keys.weight];
        let others = sum - i32::from(*weight >= 0) * bias_sum;
        if (others + bias_sum >= 0) != (others >= 0) {
            step(weight, (bias_sum >= 0) == taken, self.widths.weight);
        }
        let bits = self.widths.counter;
        step(&mut self.bias[keys.bias], taken, bits);
        step(&mut self.bias_sk[keys.bias_sk], taken, bits);
        step(&mut self.bias_bank[keys.bias_bank], taken, bits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PC: u64 = 0x8000_1004;
    const FORWARD: u64 = PC + 0x40;
    const BACKWARD: u64 = PC - 0x40;

    fn tage(conf: TageConfLevel, pred_taken: bool) -> TageScMeta {
        TageScMeta {
            conf,
            provider_bank: 3,
            alt_bank_present: true,
            pred_taken,
            provider_disagrees_with_alt: false,
        }
    }

    fn predict(sc: &StatCorrector, target: u64, tage: TageScMeta) -> ScPrediction {
        sc.predict(PC, target, 0, tage, tage.pred_taken)
    }

    #[test]
    fn an_untrained_corrector_keeps_the_prediction_before_it() {
        let sc = StatCorrector::new(&ScConfig::default());

        for before in [false, true] {
            let prediction = predict(&sc, FORWARD, tage(TageConfLevel::Low, before));

            assert_eq!(prediction.taken(), before);
        }
    }

    #[test]
    fn a_branch_tage_keeps_getting_wrong_is_corrected() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let wrong = tage(TageConfLevel::Low, true);

        for _ in 0..64 {
            let prediction = predict(&sc, FORWARD, wrong);
            sc.speculate(&prediction, false);
            sc.update(&prediction, false);
        }

        assert!(!predict(&sc, FORWARD, wrong).taken());
    }

    #[test]
    fn a_small_sum_does_not_overrule_confident_tage() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let prediction = predict(&sc, FORWARD, tage(TageConfLevel::High, true));
        sc.second_chooser = -1;

        let taken = sc.choose(TageConfLevel::High, true, -1, prediction.threshold);

        assert!(taken, "|sum| below a quarter of the threshold keeps TAGE");
    }

    #[test]
    fn a_squash_restores_every_history() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let meta = tage(TageConfLevel::Low, true);
        for i in 0..20 {
            let prediction = predict(&sc, BACKWARD, meta);
            sc.speculate(&prediction, i % 3 != 0);
        }
        let before = sc.histories(PC, 0);

        let mut squashed = Vec::new();
        for i in 0..10 {
            let target = if i % 2 == 0 { BACKWARD } else { FORWARD };
            let prediction = predict(&sc, target, meta);
            sc.speculate(&prediction, i % 4 != 0);
            squashed.push(prediction);
        }
        for prediction in squashed.iter().rev() {
            sc.squash(prediction);
        }

        assert_eq!(sc.histories(PC, 0), before);
    }

    #[test]
    fn the_imli_counter_counts_taken_backward_branches_until_the_loop_exits() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let meta = tage(TageConfLevel::Low, true);

        for _ in 0..5 {
            let prediction = predict(&sc, BACKWARD, meta);
            sc.speculate(&prediction, true);
        }
        let forward = predict(&sc, FORWARD, meta);
        sc.speculate(&forward, true);
        let counted = sc.imli_count;
        let exit = predict(&sc, BACKWARD, meta);
        sc.speculate(&exit, false);

        assert_eq!((counted, sc.imli_count), (5, 0));
    }

    #[test]
    fn training_reaches_the_counters_the_prediction_read() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let meta = tage(TageConfLevel::Low, true);
        let prediction = predict(&sc, BACKWARD, meta);
        sc.speculate(&prediction, false);
        let mut younger = Vec::new();
        for _ in 0..8 {
            let later = predict(&sc, BACKWARD, meta);
            sc.speculate(&later, true);
            younger.push(later);
        }

        sc.update(&prediction, false);
        for later in younger.iter().rev() {
            sc.squash(later);
        }
        sc.squash(&prediction);
        let again = predict(&sc, BACKWARD, meta);

        assert!(again.sum < prediction.sum, "{} then {}", prediction.sum, again.sum);
    }

    #[test]
    fn stepped_counters_saturate_at_their_width() {
        assert_eq!(stepped(31, true, 6), 31);
        assert_eq!(stepped(-32, false, 6), -32);
        assert_eq!(stepped(0, true, 6), 1);
    }
}
