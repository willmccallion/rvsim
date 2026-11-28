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
use crate::common::VirtAddr;
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

/// The most GEHL components: the global, backward, path, IMLI and IMLI
/// history ones, and one per local history.
const MAX_GEHLS: usize = 9;
const _: () = assert!(MAX_GEHLS == 5 + MAX_LOCAL_HISTORIES);

/// What one component read for a prediction: the centred sum of its
/// counters and whether its weight doubled that sum's vote. Training uses
/// these, not the counters as they are by commit, as hardware that carries
/// a prediction's reads to commit does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ComponentRead {
    sum: i32,
    doubled: bool,
}

impl ComponentRead {
    const fn vote(self) -> i32 {
        (1 + self.doubled as i32) * self.sum
    }

    /// Whether this component's vote decided the sign of `total`, gem5's
    /// `(xsum + percsum >= 0) != (xsum >= 0)`.
    const fn decided(self, total: i32) -> bool {
        let others = total - self.doubled as i32 * self.sum;
        (others + self.sum >= 0) != (others >= 0)
    }

    /// Steps `weight` toward doubling the vote when the sum read agreed
    /// with the outcome and the vote decided `total`'s sign.
    fn train_weight(self, weight: &mut i8, total: i32, taken: bool, bits: u32) {
        if self.decided(total) {
            step(weight, (self.sum >= 0) == taken, bits);
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

    /// What the component reads for its vote, `StatisticalCorrector::gPredict`.
    fn read(
        &self,
        pc: u64,
        before_sc: bool,
        histories: &ScHistories,
        weight: usize,
    ) -> ComponentRead {
        let (pc, history) = (self.hashed_pc(pc, before_sc), histories.of(self.source));
        let sum = (0..self.tables.len())
            .map(|table| centred(self.tables[table][self.index(table, pc, history)]))
            .sum();
        ComponentRead { sum, doubled: self.weights[weight] >= 0 }
    }

    /// `StatisticalCorrector::gUpdate`: steps the counters `prediction` read
    /// toward the outcome and moves the weight when the vote `read` gave
    /// decided the total's sign.
    fn train(
        &mut self,
        prediction: &ScPrediction,
        read: ComponentRead,
        taken: bool,
        keys: &Keys,
        bits: &Widths,
    ) {
        let pc = self.hashed_pc(prediction.pc.val(), prediction.before_sc);
        let history = prediction.histories.of(self.source);
        for table in 0..self.tables.len() {
            let index = self.index(table, pc, history);
            step(&mut self.tables[table][index], taken, bits.counter);
        }
        read.train_weight(&mut self.weights[keys.weight], prediction.sum, taken, bits.weight);
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
    pc: VirtAddr,
    /// The branch goes backward when taken.
    backward: bool,
    histories: ScHistories,
    tage: TageScMeta,
    before_sc: bool,
    /// What the bias tables read.
    bias: ComponentRead,
    /// What each GEHL component read, in the corrector's order.
    gehls: [ComponentRead; MAX_GEHLS],
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
        pc: VirtAddr,
        target: VirtAddr,
        path: u64,
        tage: TageScMeta,
        before_sc: bool,
    ) -> ScPrediction {
        let histories = self.histories(pc.val(), path);
        let keys = self.keys(pc.val(), &tage, before_sc);
        let bias = self.bias_read(&keys);
        let mut gehls = [ComponentRead::default(); MAX_GEHLS];
        for (read, gehl) in gehls.iter_mut().zip(&self.gehls) {
            *read = gehl.read(pc.val(), before_sc, &histories, keys.weight);
        }
        let sum = bias.vote() + gehls.iter().map(|read| read.vote()).sum::<i32>();
        let threshold = self.threshold(&keys);
        ScPrediction {
            pc,
            backward: target < pc,
            histories,
            tage,
            before_sc,
            bias,
            gehls,
            sum,
            threshold,
            taken: self.choose(tage.conf, before_sc, sum, threshold),
        }
    }

    /// What the three bias tables read at `keys`.
    fn bias_read(&self, keys: &Keys) -> ComponentRead {
        let sum = centred(self.bias[keys.bias])
            + centred(self.bias_sk[keys.bias_sk])
            + centred(self.bias_bank[keys.bias_bank]);
        ComponentRead { sum, doubled: self.bias_weights[keys.weight] >= 0 }
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
            local.push(prediction.pc.val(), taken);
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
            local.restore(prediction.pc.val(), history);
        }
    }

    /// Trains on a committed branch: `StatisticalCorrector::condBranchUpdate`.
    pub fn update(&mut self, prediction: &ScPrediction, taken: bool) {
        let keys = self.keys(prediction.pc.val(), &prediction.tage, prediction.before_sc);
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
        self.train_bias(&keys, prediction, taken);
        let widths = self.widths;
        for (gehl, &read) in self.gehls.iter_mut().zip(&prediction.gehls) {
            gehl.train(prediction, read, taken, &keys, &widths);
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

    /// Steps the bias counters `prediction` read toward the outcome, and
    /// their weight as a GEHL component's.
    fn train_bias(&mut self, keys: &Keys, prediction: &ScPrediction, taken: bool) {
        let weight = &mut self.bias_weights[keys.weight];
        prediction.bias.train_weight(weight, prediction.sum, taken, self.widths.weight);
        let bits = self.widths.counter;
        step(&mut self.bias[keys.bias], taken, bits);
        step(&mut self.bias_sk[keys.bias_sk], taken, bits);
        step(&mut self.bias_bank[keys.bias_bank], taken, bits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PC: VirtAddr = VirtAddr::new(0x8000_1004);
    const FORWARD: VirtAddr = VirtAddr::new(PC.val() + 0x40);
    const BACKWARD: VirtAddr = VirtAddr::new(PC.val() - 0x40);

    fn tage(conf: TageConfLevel, pred_taken: bool) -> TageScMeta {
        TageScMeta {
            conf,
            provider_bank: 3,
            alt_bank_present: true,
            pred_taken,
            provider_disagrees_with_alt: false,
        }
    }

    fn predict(sc: &StatCorrector, target: VirtAddr, tage: TageScMeta) -> ScPrediction {
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
        let before = sc.histories(PC.val(), 0);

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

        assert_eq!(sc.histories(PC.val(), 0), before);
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

    #[test]
    fn a_counter_steps_down_by_one_within_its_range() {
        assert_eq!(stepped(-5, false, 6), -6);
        assert_eq!(stepped(5, false, 6), 4);
    }

    #[test]
    fn a_centred_counter_is_twice_its_value_plus_one() {
        assert_eq!([centred(0), centred(-1), centred(3), centred(-4)], [1, -1, 7, -7]);
    }

    #[test]
    fn a_low_mask_sets_that_many_low_bits() {
        assert_eq!([low_mask(0), low_mask(8), low_mask(64)], [0, 0xFF, u64::MAX]);
    }

    #[test]
    fn the_pc_hash_folds_the_pc_over_itself_shifted_right_two() {
        assert_eq!(pc_hash(0x1234), 0x16B9);
        assert_eq!(pc_hash(0xDEAD_BEEC), 0xE906_D157);
    }

    #[test]
    fn each_source_reads_its_own_history() {
        let histories = ScHistories {
            global: 1,
            backward: 2,
            path: 3,
            local: [4, 5, 6, 7],
            imli_count: 8,
            imli_history: 9,
        };
        let sources =
            [Source::Global, Source::Backward, Source::Path, Source::Local(1), Source::Imli];

        let read: Vec<u64> = sources.iter().map(|&source| histories.of(source)).collect();

        assert_eq!(read, [1, 2, 3, 5, 8]);
        assert_eq!(histories.of(Source::ImliHistory), 9);
    }

    fn gehl(source: Source, lengths: &[u32], log_entries: u32, weights: usize) -> Gehl {
        let config = GehlConfig { lengths: lengths.to_vec(), log_entries, weight_init: 0 };
        Gehl::new(source, &config, false, weights)
    }

    #[test]
    fn a_gehl_table_starts_with_alternating_counters_but_for_an_unpaired_last_entry() {
        assert_eq!(Gehl::initial_table(1), [0]);
        assert_eq!(Gehl::initial_table(3), [-1, 0, 0]);
        assert_eq!(Gehl::initial_table(4), [-1, 0, -1, 0]);
    }

    #[test]
    fn only_the_backward_component_hashes_the_prediction_into_its_pc() {
        let backward = gehl(Source::Backward, &[4], 4, 1);
        let global = gehl(Source::Global, &[4], 4, 1);

        assert_eq!(backward.hashed_pc(5, true), 11);
        assert_eq!(backward.hashed_pc(5, false), 10);
        assert_eq!(global.hashed_pc(5, true), 5);
    }

    #[test]
    fn a_gehl_index_hashes_the_pc_with_the_history_shifted_per_table() {
        let component = gehl(Source::Global, &[64; 8], 20, 1);
        let cases = [
            (0, 0x97_3CEB_3FFC, 0x2163_6369_8B52_9B4A, 660_734),
            (1, 0xEA_5EB5_61A4, 0x795B_929E_9A9A_80FD, 878_395),
            (2, 0x94_A02F_34A4, 0x9B08_923D_10C6_7FD9, 526_253),
            (3, 0xE8_035E_FA24, 0x781F_9C58_D664_5FA9, 837_262),
            (4, 0x8D_4265_0644, 0x3116_2427_3BFD_1D33, 646_380),
            (5, 0xB7_FEE2_9474, 0x8A7D_43B5_7863_3074, 103_648),
            (6, 0x8C_D622_5674, 0x65AA_9C82_79F2_48B0, 1_048_442),
            (7, 0xDC_A399_F828, 0x3B5F_3D86_268E_CC45, 295_395),
        ];

        for (table, pc, history, index) in cases {
            assert_eq!(component.index(table, pc, history), index, "table {table}");
        }
    }

    #[test]
    fn a_gehl_vote_doubles_for_a_non_negative_weight() {
        let mut component = gehl(Source::Global, &[4, 4], 4, 2);
        component.tables[0][0] = 2;
        component.tables[1][0] = -1;
        component.weights[0] = 0;
        component.weights[1] = -1;

        let histories = ScHistories::default();

        assert_eq!(component.read(0, false, &histories, 0).vote(), 8);
        assert_eq!(component.read(0, false, &histories, 1).vote(), 4);
    }

    #[test]
    fn only_the_imli_history_weight_stays_out_of_the_threshold() {
        let counts = |source| gehl(source, &[4], 4, 1).weight_counts_in_threshold();

        assert!(counts(Source::Global));
        assert!(counts(Source::Imli));
        assert!(!counts(Source::ImliHistory));
    }

    const WIDTHS: Widths =
        Widths { counter: 6, weight: 6, chooser: 7, threshold: 12, per_pc_threshold: 8 };
    const FIRST_KEYS: Keys =
        Keys { bias: 0, bias_sk: 0, bias_bank: 0, per_pc_threshold: 0, weight: 0 };

    /// A two-table global component whose first entries hold `a` and `b`.
    fn gehl_holding(a: i8, b: i8, weight: i8) -> Gehl {
        let mut component = gehl(Source::Global, &[4, 4], 4, 1);
        component.tables[0][0] = a;
        component.tables[1][0] = b;
        component.weights[0] = weight;
        component
    }

    fn trained_gehl(a: i8, b: i8, weight: i8, total: i32, taken: bool) -> (i8, i8, i8) {
        let mut component = gehl_holding(a, b, weight);
        let prediction = scored(total, 40, true, TageConfLevel::Low);
        let read = component.read(0, true, &prediction.histories, 0);

        component.train(&prediction, read, taken, &FIRST_KEYS, &WIDTHS);

        (component.weights[0], component.tables[0][0], component.tables[1][0])
    }

    /// The vote the prediction read (sum -4, doubled) decided a total of
    /// -1; the counters another branch moved since would not have.
    #[test]
    fn a_gehl_trains_its_weight_on_what_its_prediction_read() {
        let mut component = gehl_holding(3, -6, 0);
        let prediction = scored(-1, 40, true, TageConfLevel::Low);
        let read = component.read(0, true, &prediction.histories, 0);
        component.tables[0][0] = 6;
        component.tables[1][0] = 5;

        component.train(&prediction, read, false, &FIRST_KEYS, &WIDTHS);

        assert_eq!(component.weights[0], 1);
    }

    #[test]
    fn a_gehl_trains_its_counters_and_moves_its_weight_when_its_vote_decided_the_sign() {
        let rows = [
            ((1, -5, 2, 6, true), (2, 2, -4)),
            ((-3, -3, 0, -30, true), (0, -2, -2)),
            ((-5, -3, 1, -18, true), (1, -4, -2)),
            ((3, -6, 0, 0, false), (0, 2, -7)),
            ((6, 5, 0, 21, false), (-1, 5, 4)),
        ];

        for ((a, b, weight, total, taken), expected) in rows {
            let row = (a, b, weight, total, taken);
            assert_eq!(trained_gehl(a, b, weight, total, taken), expected, "{row:?}");
        }
    }

    fn local_histories(mix_pc: bool) -> LocalHistories {
        LocalHistories::new(&LocalGehlConfig {
            histories: 16,
            index_shift: 2,
            mix_pc,
            gehl: GehlConfig::default(),
        })
    }

    #[test]
    fn a_local_history_is_picked_by_the_pc_folded_over_itself() {
        let locals = local_histories(false);

        assert_eq!(locals.entry(0x35), 8);
    }

    #[test]
    fn a_local_history_shifts_in_each_outcome() {
        let mut locals = local_histories(false);

        for taken in [true, true, false] {
            locals.push(0x35, taken);
        }

        assert_eq!(locals.get(0x35), 0b110);
        assert_eq!(locals.get(0x36), 0, "another branch's history is untouched");
    }

    #[test]
    fn a_mixing_local_history_xors_the_low_pc_bits_into_each_update() {
        let mut locals = local_histories(true);

        locals.push(0x35, true);
        locals.push(0x35, false);

        assert_eq!(locals.get(0x35), 13);
    }

    fn bias_tables(counter_bits: u32) -> [Vec<i8>; 3] {
        StatCorrector::initial_bias(3, counter_bits).into()
    }

    #[test]
    fn the_bias_tables_start_agreeing_with_the_prediction_before_the_corrector() {
        let [bias, bias_sk, bias_bank] = bias_tables(6);

        assert_eq!(bias[..4], [-32, 31, -1, 0]);
        assert_eq!(bias_sk[..4], [-8, 7, -32, 31]);
        assert_eq!(bias_bank, bias);
        assert_eq!(bias[4..], bias[..4], "the pattern repeats every four entries");
    }

    #[test]
    fn the_bias_tables_start_at_the_extremes_of_a_narrower_counter() {
        let [bias, bias_sk, _] = bias_tables(4);

        assert_eq!(bias[..4], [-8, 7, -1, 0]);
        assert_eq!(bias_sk[..4], [-2, 1, -8, 7]);
    }

    fn scored(sum: i32, threshold: i32, before_sc: bool, conf: TageConfLevel) -> ScPrediction {
        ScPrediction {
            pc: VirtAddr::new(0),
            backward: false,
            histories: ScHistories::default(),
            tage: tage(conf, before_sc),
            before_sc,
            bias: ComponentRead::default(),
            gehls: [ComponentRead::default(); MAX_GEHLS],
            sum,
            threshold,
            taken: before_sc,
        }
    }

    const THRESHOLD: i32 = 40;

    fn chosen(conf: TageConfLevel, sum: i32, first: i8, second: i8) -> bool {
        let mut sc = StatCorrector::new(&ScConfig::default());
        sc.first_chooser = first;
        sc.second_chooser = second;
        sc.choose(conf, true, sum, THRESHOLD)
    }

    #[test]
    fn a_corrector_that_agrees_with_the_prediction_keeps_it() {
        let sc = StatCorrector::new(&ScConfig::default());
        assert!(chosen(TageConfLevel::Low, 5, 0, 0));
        assert!(!sc.choose(TageConfLevel::Low, false, -5, THRESHOLD));
    }

    #[test]
    fn a_high_confidence_prediction_resists_a_sum_below_a_quarter_of_the_threshold() {
        assert!(chosen(TageConfLevel::High, -9, -1, -1));
        assert!(chosen(TageConfLevel::High, -9, 0, 0));
    }

    #[test]
    fn a_high_confidence_prediction_yields_between_a_quarter_and_half_when_the_chooser_says_so() {
        assert!(chosen(TageConfLevel::High, -10, 0, 0));
        assert!(!chosen(TageConfLevel::High, -10, 0, -1));
        assert!(chosen(TageConfLevel::High, -19, 0, 0));
        assert!(!chosen(TageConfLevel::High, -19, 0, -1));
    }

    #[test]
    fn a_high_confidence_prediction_yields_to_a_sum_of_half_the_threshold() {
        assert!(!chosen(TageConfLevel::High, -20, 0, 0));
        assert!(!chosen(TageConfLevel::High, -45, 0, 0));
    }

    #[test]
    fn a_medium_confidence_prediction_yields_below_a_quarter_only_when_the_chooser_says_so() {
        assert!(chosen(TageConfLevel::Medium, -9, 0, 0));
        assert!(!chosen(TageConfLevel::Medium, -9, -1, 0));
    }

    #[test]
    fn a_medium_confidence_prediction_yields_to_a_sum_of_a_quarter_of_the_threshold() {
        assert!(!chosen(TageConfLevel::Medium, -10, 0, 0));
        assert!(!chosen(TageConfLevel::Medium, -15, 0, 0));
    }

    #[test]
    fn a_low_confidence_prediction_always_yields_to_the_corrector() {
        assert!(!chosen(TageConfLevel::Low, -1, 0, 0));
        assert!(!chosen(TageConfLevel::None, -1, 0, 0));
    }

    #[test]
    fn a_prediction_keeps_the_histories_it_read() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let meta = tage(TageConfLevel::Low, true);
        let prediction = predict(&sc, FORWARD, meta);
        sc.speculate(&prediction, true);

        let read = predict(&sc, FORWARD, meta).histories;

        assert_eq!(read.global, 1);
        assert_eq!(read.imli_history, 1);
        assert_ne!(read, ScHistories::default());
    }

    #[test]
    fn a_prediction_keeps_the_local_history_of_its_own_branch() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let meta = tage(TageConfLevel::Low, true);
        let prediction = predict(&sc, FORWARD, meta);
        sc.speculate(&prediction, true);

        let read = predict(&sc, FORWARD, meta).histories;

        assert_eq!(read.local[0], 1);
    }

    #[test]
    fn a_prediction_sums_its_three_bias_counters_doubled_by_a_non_negative_weight() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        sc.gehls.clear();
        let meta = tage(TageConfLevel::Low, true);
        let keys = sc.keys(PC.val(), &meta, true);
        sc.bias[keys.bias] = 2;
        sc.bias_sk[keys.bias_sk] = -2;
        sc.bias_bank[keys.bias_bank] = 3;

        sc.bias_weights[keys.weight] = 0;
        let doubled = predict(&sc, FORWARD, meta).sum;
        sc.bias_weights[keys.weight] = -1;
        let single = predict(&sc, FORWARD, meta).sum;

        assert_eq!((doubled, single), (18, 9));
    }

    #[test]
    fn a_prediction_adds_each_gehl_components_vote_to_the_bias_sum() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let meta = tage(TageConfLevel::Low, true);
        let with_gehls = predict(&sc, FORWARD, meta);
        let histories = sc.histories(PC.val(), 0);
        let keys = sc.keys(PC.val(), &meta, true);
        let votes: i32 = sc
            .gehls
            .iter()
            .map(|gehl| gehl.read(PC.val(), true, &histories, keys.weight).vote())
            .sum();
        sc.gehls.clear();
        let bias_only = predict(&sc, FORWARD, meta);

        assert_eq!(with_gehls.sum, bias_only.sum + votes);
    }

    #[test]
    fn a_branch_to_its_own_address_is_not_a_backward_branch() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let meta = tage(TageConfLevel::Low, true);

        let to_itself = predict(&sc, PC, meta);
        sc.speculate(&to_itself, true);

        assert!(!to_itself.backward);
        assert_eq!(sc.imli_count, 0);
    }

    #[test]
    fn the_corrector_shifts_each_outcome_into_the_global_and_imli_histories() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let meta = tage(TageConfLevel::Low, true);

        for taken in [true, false, true] {
            let prediction = predict(&sc, FORWARD, meta);
            sc.speculate(&prediction, taken);
        }

        assert_eq!(sc.global, 0b101);
        assert_eq!(sc.imli_histories[0], 0b101);
        assert_eq!(sc.locals[0].get(PC.val()), 0b101);
    }

    #[test]
    fn the_backward_history_records_only_taken_backward_branches() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let meta = tage(TageConfLevel::Low, true);
        for (target, taken) in
            [(BACKWARD, true), (BACKWARD, false), (FORWARD, true), (BACKWARD, true)]
        {
            let prediction = predict(&sc, target, meta);
            sc.speculate(&prediction, taken);
        }

        assert_eq!(sc.backward, 0b1001);
    }

    /// What TAGE knew of a branch at `pc`, and the table keys that gives.
    struct KeyCase {
        pc: u64,
        conf: TageConfLevel,
        disagrees: bool,
        bank: usize,
        alt: bool,
        before: bool,
        keys: [usize; 5],
    }

    const KEY_CASES: [KeyCase; 16] = [
        KeyCase {
            pc: 0xd73cf256c,
            conf: TageConfLevel::Low,
            disagrees: false,
            bank: 59,
            alt: false,
            before: true,
            keys: [221, 229, 117, 55, 7],
        },
        KeyCase {
            pc: 0x9dae44550,
            conf: TageConfLevel::Medium,
            disagrees: true,
            bank: 51,
            alt: false,
            before: false,
            keys: [16, 20, 208, 4, 4],
        },
        KeyCase {
            pc: 0x2cb008850,
            conf: TageConfLevel::High,
            disagrees: true,
            bank: 19,
            alt: true,
            before: false,
            keys: [16, 198, 90, 4, 4],
        },
        KeyCase {
            pc: 0xbe3eff9c0,
            conf: TageConfLevel::None,
            disagrees: false,
            bank: 38,
            alt: false,
            before: false,
            keys: [192, 156, 144, 48, 0],
        },
        KeyCase {
            pc: 0xba7677794,
            conf: TageConfLevel::Low,
            disagrees: false,
            bank: 10,
            alt: false,
            before: false,
            keys: [196, 40, 164, 49, 1],
        },
        KeyCase {
            pc: 0x102b9388,
            conf: TageConfLevel::Medium,
            disagrees: true,
            bank: 56,
            alt: true,
            before: true,
            keys: [169, 25, 233, 42, 2],
        },
        KeyCase {
            pc: 0x576c468ac,
            conf: TageConfLevel::High,
            disagrees: true,
            bank: 53,
            alt: true,
            before: true,
            keys: [29, 59, 91, 7, 7],
        },
        KeyCase {
            pc: 0x74b4d8474,
            conf: TageConfLevel::None,
            disagrees: true,
            bank: 5,
            alt: true,
            before: true,
            keys: [165, 149, 153, 41, 1],
        },
        KeyCase {
            pc: 0x8ff666588,
            conf: TageConfLevel::Low,
            disagrees: false,
            bank: 53,
            alt: true,
            before: true,
            keys: [169, 121, 221, 42, 2],
        },
        KeyCase {
            pc: 0x3c20ba2c0,
            conf: TageConfLevel::Medium,
            disagrees: false,
            bank: 1,
            alt: true,
            before: false,
            keys: [192, 44, 8, 48, 0],
        },
        KeyCase {
            pc: 0x166809a10,
            conf: TageConfLevel::High,
            disagrees: false,
            bank: 24,
            alt: true,
            before: true,
            keys: [81, 227, 107, 20, 4],
        },
        KeyCase {
            pc: 0xaf5570ec,
            conf: TageConfLevel::None,
            disagrees: true,
            bank: 59,
            alt: false,
            before: true,
            keys: [93, 189, 113, 23, 7],
        },
        KeyCase {
            pc: 0xbfaf8cda8,
            conf: TageConfLevel::Low,
            disagrees: true,
            bank: 4,
            alt: false,
            before: true,
            keys: [11, 121, 21, 2, 2],
        },
        KeyCase {
            pc: 0x4acc6d8f0,
            conf: TageConfLevel::Medium,
            disagrees: true,
            bank: 19,
            alt: true,
            before: false,
            keys: [48, 76, 88, 12, 4],
        },
        KeyCase {
            pc: 0xec20ef164,
            conf: TageConfLevel::High,
            disagrees: true,
            bank: 15,
            alt: false,
            before: true,
            keys: [245, 135, 195, 61, 5],
        },
        KeyCase {
            pc: 0xc77064c2c,
            conf: TageConfLevel::None,
            disagrees: true,
            bank: 43,
            alt: false,
            before: true,
            keys: [157, 113, 49, 39, 7],
        },
    ];

    #[test]
    fn a_branchs_table_keys_hash_its_pc_with_what_tage_knew_of_it() {
        let sc = StatCorrector::new(&ScConfig::default());

        for KeyCase { pc, conf, disagrees, bank, alt, before, keys: expected } in KEY_CASES {
            let meta = TageScMeta {
                conf,
                provider_bank: bank,
                alt_bank_present: alt,
                pred_taken: before,
                provider_disagrees_with_alt: disagrees,
            };

            let keys = sc.keys(pc, &meta, before);

            let found =
                [keys.bias, keys.bias_sk, keys.bias_bank, keys.per_pc_threshold, keys.weight];
            assert_eq!(found, expected, "pc {pc:#x} {conf:?}");
        }
    }

    #[test]
    fn the_threshold_adds_a_step_for_each_component_doubling_its_vote() {
        let mut sc = StatCorrector::new(&ScConfig::default());
        let keys = sc.keys(PC.val(), &tage(TageConfLevel::Low, true), true);

        assert_eq!(sc.threshold(&keys), 35 + 12 * 7);

        sc.per_pc_thresholds[keys.per_pc_threshold] = 5;
        sc.gehls[1].weights[keys.weight] = -1;
        assert_eq!(sc.threshold(&keys), 35 + 5 + 12 * 6);
    }

    fn corrected(sum: i32, before_sc: bool, conf: TageConfLevel) -> (StatCorrector, ScPrediction) {
        let sc = StatCorrector::new(&ScConfig::default());
        let mut prediction = scored(sum, THRESHOLD, before_sc, conf);
        prediction.pc = PC;
        (sc, prediction)
    }

    fn choosers_after(conf: TageConfLevel, sum: i32, taken: bool) -> (i8, i8) {
        let (mut sc, prediction) = corrected(sum, true, conf);

        sc.train_choosers(&prediction, taken);

        (sc.first_chooser, sc.second_chooser)
    }

    #[test]
    fn the_second_chooser_learns_from_high_confidence_predictions_in_its_zone() {
        for (sum, taken, expected) in
            [(-5, true, 0), (-10, true, 1), (-19, false, -1), (-20, true, 0), (-30, true, 0)]
        {
            let (first, second) = choosers_after(TageConfLevel::High, sum, taken);

            assert_eq!((first, second), (0, expected), "sum {sum}");
        }
    }

    #[test]
    fn the_first_chooser_learns_from_medium_confidence_predictions_below_a_quarter() {
        for (sum, taken, expected) in
            [(-9, true, 1), (-9, false, -1), (-10, true, 0), (-15, true, 0)]
        {
            let (first, second) = choosers_after(TageConfLevel::Medium, sum, taken);

            assert_eq!((first, second), (expected, 0), "sum {sum}");
        }
    }

    #[test]
    fn a_low_confidence_prediction_trains_neither_chooser() {
        assert_eq!(choosers_after(TageConfLevel::Low, -9, true), (0, 0));
        assert_eq!(choosers_after(TageConfLevel::Low, -15, true), (0, 0));
    }

    #[test]
    fn an_agreeing_corrector_does_not_train_the_choosers() {
        let (mut sc, prediction) = corrected(5, true, TageConfLevel::Medium);

        sc.update(&prediction, true);

        assert_eq!((sc.first_chooser, sc.second_chooser), (0, 0));
    }

    #[test]
    fn a_disagreeing_corrector_trains_the_choosers_on_commit() {
        let (mut sc, prediction) = corrected(-5, true, TageConfLevel::Medium);

        sc.update(&prediction, true);

        assert_eq!(sc.first_chooser, 1);
    }

    fn thresholds_after(sum: i32, taken: bool) -> (i32, i32) {
        let (mut sc, prediction) = corrected(sum, true, TageConfLevel::Low);
        let keys = sc.keys(PC.val(), &prediction.tage, true);

        sc.update(&prediction, taken);

        (sc.threshold, sc.per_pc_thresholds[keys.per_pc_threshold])
    }

    #[test]
    fn a_wrong_corrector_raises_both_thresholds() {
        assert_eq!(thresholds_after(-5, true), (281, 1));
    }

    #[test]
    fn a_right_but_weak_corrector_lowers_both_thresholds() {
        assert_eq!(thresholds_after(THRESHOLD - 1, true), (279, -1));
    }

    #[test]
    fn a_right_and_confident_corrector_leaves_the_thresholds_and_counters_alone() {
        let (mut sc, prediction) = corrected(THRESHOLD, true, TageConfLevel::Low);
        let keys = sc.keys(PC.val(), &prediction.tage, true);
        let bias = sc.bias[keys.bias];

        sc.update(&prediction, true);

        assert_eq!((sc.threshold, sc.bias[keys.bias]), (35 << 3, bias));
    }

    #[test]
    fn training_steps_the_bias_counters_toward_the_outcome() {
        let (mut sc, prediction) = corrected(-5, true, TageConfLevel::Low);
        let keys = sc.keys(PC.val(), &prediction.tage, true);
        let before = [sc.bias[keys.bias], sc.bias_sk[keys.bias_sk], sc.bias_bank[keys.bias_bank]];

        sc.update(&prediction, true);

        let after = [sc.bias[keys.bias], sc.bias_sk[keys.bias_sk], sc.bias_bank[keys.bias_bank]];
        assert_eq!(after, before.map(|ctr| stepped(i32::from(ctr), true, 6) as i8));
    }

    fn trained_bias(row: (i8, i8, i8, i8, i32, bool)) -> (i8, i8, i8, i8) {
        let (a, b, c, weight, total, taken) = row;
        let mut sc = StatCorrector::new(&ScConfig::default());
        let keys = Keys { bias: 1, bias_sk: 2, bias_bank: 3, per_pc_threshold: 0, weight: 0 };
        sc.bias[1] = a;
        sc.bias_sk[2] = b;
        sc.bias_bank[3] = c;
        sc.bias_weights[0] = weight;
        let mut prediction = scored(total, 40, true, TageConfLevel::Low);
        prediction.bias = sc.bias_read(&keys);

        sc.train_bias(&keys, &prediction, taken);

        (sc.bias_weights[0], sc.bias[1], sc.bias_sk[2], sc.bias_bank[3])
    }

    #[test]
    fn the_bias_tables_train_their_counters_and_weight_like_a_gehl_component() {
        let rows = [
            ((6, 5, 5, 1, -15, false), (1, 5, 4, 4)),
            ((-5, -5, 6, 1, 12, false), (1, -6, -6, 5)),
            ((4, -2, 1, 0, -12, true), (0, 5, -1, 2)),
            ((-6, -6, 1, 2, -3, false), (3, -7, -7, 0)),
            ((-2, -1, -4, -1, 9, true), (-2, -1, 0, -3)),
        ];

        for (row, expected) in rows {
            assert_eq!(trained_bias(row), expected, "{row:?}");
        }
    }
}
