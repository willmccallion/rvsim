//! Seznec's loop predictor, as TAGE-SC-L composes it.
//!
//! It learns each counted loop's iteration count and, once confident,
//! predicts the exit exactly. A use counter (`WITHLOOP`) learns whether its
//! predictions beat the prediction they would replace. Fetch predicts
//! ahead of commit, so each entry keeps a speculative iteration count that
//! predictions advance and a squash restores; commit trains with the
//! committed count, as gem5's `LoopPredictor` does.

use crate::config::LoopConfig;
use crate::uarch::bpred::components::xorshift64;

/// An entry of the loop table.
#[derive(Clone, Copy, Debug, Default)]
struct LoopEntry {
    tag: u16,
    /// Iterations of the loop's last full run; 0 while not yet learnt.
    num_iter: u16,
    /// Iterations committed in the current run.
    current_iter: u16,
    /// Iterations predicted in the current run, ahead of commit.
    spec_iter: u16,
    confidence: u8,
    age: u8,
    /// The direction the loop body takes (with the direction bit).
    dir: bool,
}

/// What a loop prediction read, for the branch to speculate, repair and
/// train with.
#[derive(Clone, Copy, Debug)]
pub struct LoopPrediction {
    set: usize,
    set_hash: usize,
    tag: u16,
    /// The way that matched.
    way: Option<usize>,
    /// The matching entry is confident enough to predict.
    valid: bool,
    /// Its prediction.
    taken: bool,
    /// The entry's speculative iteration count before this branch.
    spec_iter_before: u16,
}

impl LoopPrediction {
    /// The loop predictor's direction, when it is confident.
    #[must_use]
    pub const fn confident(&self) -> Option<bool> {
        if self.valid { Some(self.taken) } else { None }
    }
}

/// Seznec's loop predictor.
#[derive(Debug)]
pub struct LoopPredictor {
    table: Vec<LoopEntry>,
    config: LoopConfig,
    set_mask: usize,
    tag_mask: u16,
    iter_mask: u16,
    /// `WITHLOOP`: whether confident loop predictions are used.
    use_counter: i8,
    random: u64,
}

impl LoopPredictor {
    /// A loop predictor of `2^log_size` entries in `2^log_assoc` ways.
    pub fn new(config: &LoopConfig) -> Self {
        let log_size = config.log_size.max(config.log_assoc);
        Self {
            table: vec![LoopEntry::default(); 1 << log_size],
            set_mask: (1 << (log_size - config.log_assoc)) - 1,
            tag_mask: ((1u32 << config.tag_bits) - 1) as u16,
            iter_mask: ((1u32 << config.iter_bits) - 1) as u16,
            use_counter: -1,
            random: 0x2545_F491_4F6C_DD1D,
            config: config.clone(),
        }
    }

    /// Looks the branch at `pc` up, predicting from the speculative
    /// iteration counts.
    #[must_use]
    pub fn predict(&self, pc: u64) -> LoopPrediction {
        self.lookup(pc, true)
    }

    /// Whether confident loop predictions are currently used.
    #[must_use]
    pub const fn in_use(&self) -> bool {
        self.use_counter >= 0
    }

    /// Advances the matching entry's speculative iteration count with the
    /// direction fetch follows.
    pub fn speculate(&mut self, prediction: &LoopPrediction, taken: bool) {
        let Some(index) = self.hit_index(prediction) else { return };
        let (iter_mask, body) = (self.iter_mask, self.body_direction(&self.table[index]));
        let entry = &mut self.table[index];
        entry.spec_iter = if taken == body { (entry.spec_iter + 1) & iter_mask } else { 0 };
    }

    /// Restores the speculative iteration count `prediction` advanced.
    pub fn squash(&mut self, prediction: &LoopPrediction) {
        if let Some(index) = self.hit_index(prediction) {
            self.table[index].spec_iter = prediction.spec_iter_before;
        }
    }

    /// Trains with a committed branch: `tage_taken` is TAGE's prediction
    /// and `final_taken` the one fetch followed. As gem5 does with
    /// speculation, it looks the entry up again with the committed count.
    pub fn commit(&mut self, pc: u64, taken: bool, tage_taken: bool, final_taken: bool) {
        let committed = self.lookup(pc, false);
        if committed.valid && final_taken != committed.taken {
            self.use_counter = step_signed(
                self.use_counter,
                committed.taken == taken,
                self.config.use_counter_bits,
            );
        }
        self.train(&committed, taken, tage_taken, final_taken);
    }

    fn lookup(&self, pc: u64, speculative: bool) -> LoopPrediction {
        let (set, set_hash, tag) = self.set_and_tag(pc);
        let mut prediction = LoopPrediction {
            set,
            set_hash,
            tag,
            way: None,
            valid: false,
            taken: false,
            spec_iter_before: 0,
        };
        for way in 0..self.ways() {
            let index = self.entry_index(set, set_hash, way);
            let entry = &self.table[index];
            if entry.tag != tag {
                continue;
            }
            let iter = if speculative { entry.spec_iter } else { entry.current_iter };
            let exits = iter.wrapping_add(1) == entry.num_iter;
            let body = self.body_direction(entry);
            prediction.way = Some(way);
            prediction.valid = self.confident(entry);
            prediction.taken = if exits { !body } else { body };
            prediction.spec_iter_before = entry.spec_iter;
            break;
        }
        prediction
    }

    /// `LoopPredictor::loopUpdate`: learns the iteration count, frees an
    /// entry that mispredicted, and allocates one when the prediction the
    /// branch was fetched with was wrong.
    fn train(
        &mut self,
        prediction: &LoopPrediction,
        taken: bool,
        tage_taken: bool,
        final_taken: bool,
    ) {
        let Some(index) = self.hit_index(prediction) else {
            self.allocate(prediction, taken, final_taken);
            return;
        };
        let optional_age_increment =
            self.config.optional_age_increment && self.next_random().trailing_zeros() >= 3;
        let optional_age_reset = self.config.optional_age_reset;
        let body = self.body_direction(&self.table[index]);
        let iter_mask = self.iter_mask;
        let confidence_max = max_unsigned(self.config.confidence_bits);
        let age_max = max_unsigned(self.config.age_bits);
        let entry = &mut self.table[index];
        if prediction.valid {
            if taken != prediction.taken {
                *entry = LoopEntry { tag: entry.tag, dir: entry.dir, ..LoopEntry::default() };
                return;
            }
            if prediction.taken != tage_taken || optional_age_increment {
                entry.age = (entry.age + 1).min(age_max);
            }
        }

        entry.current_iter = (entry.current_iter + 1) & iter_mask;
        if entry.current_iter > entry.num_iter {
            entry.confidence = 0;
            if entry.num_iter != 0 {
                entry.num_iter = 0;
                if optional_age_reset {
                    entry.age = 0;
                }
            }
        }

        if taken != body {
            if entry.current_iter == entry.num_iter {
                entry.confidence = (entry.confidence + 1).min(confidence_max);
                // A loop of one or two iterations is not worth predicting.
                if entry.num_iter < 3 {
                    entry.dir = taken;
                    entry.num_iter = 0;
                    entry.age = 0;
                    entry.confidence = 0;
                }
            } else if entry.num_iter == 0 {
                entry.confidence = 0;
                entry.num_iter = entry.current_iter;
            } else {
                entry.num_iter = 0;
                if optional_age_reset {
                    entry.age = 0;
                }
                entry.confidence = 0;
            }
            entry.current_iter = 0;
        }
    }

    /// Takes a free way for a branch the prediction fetch followed got
    /// wrong (with the direction bit), or a taken branch (without it),
    /// aging the ways it passes over.
    fn allocate(&mut self, prediction: &LoopPrediction, taken: bool, final_taken: bool) {
        let wants = if self.config.use_direction_bit { final_taken != taken } else { taken };
        if !wants || (self.config.restrict_allocation && self.next_random() & 3 != 0) {
            return;
        }
        let start = self.next_random() as usize;
        let ways = self.ways();
        for step in 0..ways {
            let way = (start + step) & (ways - 1);
            let index = self.entry_index(prediction.set, prediction.set_hash, way);
            let entry = &mut self.table[index];
            if entry.age == 0 {
                let iter = self.config.initial_iter;
                *entry = LoopEntry {
                    tag: prediction.tag,
                    num_iter: 0,
                    current_iter: iter,
                    spec_iter: iter,
                    confidence: 0,
                    age: self.config.initial_age,
                    dir: !taken,
                };
                return;
            }
            entry.age -= 1;
            if self.config.restrict_allocation {
                return;
            }
        }
    }

    /// `LoopPredictor::lindex` and the tag: the set, the hash spreading a
    /// set's ways, and the tag.
    const fn set_and_tag(&self, pc: u64) -> (usize, usize, u16) {
        let pc = pc as usize;
        let shifted = pc >> 2;
        let set_bits = if self.config.use_hashing { shifted ^ pc } else { shifted };
        let set = (set_bits & self.set_mask) << self.config.log_assoc;
        let pc_shift = self.config.log_size - self.config.log_assoc;
        if self.config.use_hashing {
            let set_hash = (pc >> pc_shift) & self.set_mask;
            let tag = (pc >> pc_shift) ^ (pc >> (pc_shift + self.config.tag_bits));
            (set, set_hash, tag as u16 & self.tag_mask)
        } else {
            let tag = (pc >> (2 + pc_shift)) as u16 & self.tag_mask;
            (set, 0, tag)
        }
    }

    /// `LoopPredictor::finallindex`.
    const fn entry_index(&self, set: usize, set_hash: usize, way: usize) -> usize {
        let base = if self.config.use_hashing {
            set ^ ((set_hash >> way) << self.config.log_assoc)
        } else {
            set
        };
        base + way
    }

    fn hit_index(&self, prediction: &LoopPrediction) -> Option<usize> {
        let way = prediction.way?;
        let index = self.entry_index(prediction.set, prediction.set_hash, way);
        (self.table[index].tag == prediction.tag).then_some(index)
    }

    const fn ways(&self) -> usize {
        1 << self.config.log_assoc
    }

    /// The direction of the loop body: taken, or the learnt direction with
    /// the direction bit.
    const fn body_direction(&self, entry: &LoopEntry) -> bool {
        !self.config.use_direction_bit || entry.dir
    }

    /// A saturated confidence, or with `long_loop_confidence` enough
    /// confidence over a long enough loop (TAGE-SC-L's rule).
    fn confident(&self, entry: &LoopEntry) -> bool {
        entry.confidence == max_unsigned(self.config.confidence_bits)
            || (self.config.long_loop_confidence
                && u32::from(entry.confidence) * u32::from(entry.num_iter) > 128)
    }

    /// The next value of a xorshift generator, where gem5 draws from a
    /// Mersenne twister.
    const fn next_random(&mut self) -> u64 {
        xorshift64(&mut self.random)
    }
}

const fn max_unsigned(bits: usize) -> u8 {
    ((1u16 << bits) - 1) as u8
}

/// Steps a signed `bits`-wide counter toward `up`.
const fn step_signed(ctr: i8, up: bool, bits: usize) -> i8 {
    let max = ((1i16 << (bits - 1)) - 1) as i8;
    let min = (-(1i16 << (bits - 1))) as i8;
    if up {
        if ctr < max { ctr + 1 } else { ctr }
    } else if ctr > min {
        ctr - 1
    } else {
        ctr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LoopConfig {
        LoopConfig {
            restrict_allocation: false,
            optional_age_increment: false,
            ..LoopConfig::default()
        }
    }

    /// Runs `runs` full runs of a loop at `pc` of `iterations` taken
    /// branches and one exit, committing each as it is predicted; fetch
    /// predicts taken throughout, so only the exits are mispredicted.
    fn run_loop(lp: &mut LoopPredictor, pc: u64, iterations: usize, runs: usize) {
        for _ in 0..runs {
            for i in 0..=iterations {
                let taken = i < iterations;
                let prediction = lp.predict(pc);
                lp.speculate(&prediction, taken);
                lp.commit(pc, taken, true, true);
            }
        }
    }

    #[test]
    fn a_learnt_loop_predicts_its_exit() {
        let mut lp = LoopPredictor::new(&config());
        let pc = 0x8000_1000u64;
        run_loop(&mut lp, pc, 10, 40);

        for i in 0..=10 {
            let prediction = lp.predict(pc);
            assert_eq!(prediction.confident(), Some(i < 10), "iteration {i}");
            lp.speculate(&prediction, i < 10);
        }
    }

    #[test]
    fn predictions_ahead_of_commit_count_the_iterations_in_flight() {
        let mut lp = LoopPredictor::new(&config());
        let pc = 0x8000_1000u64;
        run_loop(&mut lp, pc, 10, 40);

        for _ in 0..10 {
            let prediction = lp.predict(pc);
            lp.speculate(&prediction, true);
        }

        assert_eq!(lp.predict(pc).confident(), Some(false), "the exit, with nothing committed");
    }

    #[test]
    fn a_squash_restores_the_speculative_iteration_count() {
        let mut lp = LoopPredictor::new(&config());
        let pc = 0x8000_1000u64;
        run_loop(&mut lp, pc, 10, 40);
        let before = lp.predict(pc);

        let mut squashed = Vec::new();
        for _ in 0..4 {
            let prediction = lp.predict(pc);
            lp.speculate(&prediction, true);
            squashed.push(prediction);
        }
        for prediction in squashed.iter().rev() {
            lp.squash(prediction);
        }

        assert_eq!(lp.predict(pc).spec_iter_before, before.spec_iter_before);
    }

    #[test]
    fn loop_predictions_are_used_only_while_they_help() {
        let lp = LoopPredictor::new(&config());

        assert!(!lp.in_use(), "WITHLOOP starts negative");
    }
}
