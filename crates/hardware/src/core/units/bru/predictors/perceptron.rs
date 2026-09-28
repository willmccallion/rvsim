//! Perceptron Branch Predictor.
//!
//! Uses a single-layer perceptron neural network to predict branch direction.
//! Instead of saturating counters, it uses a table of weight vectors. The
//! prediction is the dot product of the weights and the history vector.

use crate::config::PerceptronConfig;
use crate::core::units::bru::direction::{BranchClass, DirectionPredictor, Jump, Retired};

/// Coefficient used to calculate the training threshold.
const THETA_COEFF: f64 = 1.93;
/// Bias used to calculate the training threshold.
const THETA_BIAS: f64 = 14.0;

/// Perceptron Predictor structure.
#[derive(Debug)]
pub struct PerceptronPredictor {
    /// Global history as fetched, which predictions use.
    ghr: u64,
    /// Table of weights (flattened).
    table: Vec<i8>,
    /// Length of the history vector.
    history_length: usize,
    /// Mask for indexing the table.
    table_mask: usize,
    /// Size of a single row in the table (history length + bias).
    row_size: usize,
    /// Training threshold (theta).
    threshold: i32,
}

/// What a perceptron prediction was made with.
#[derive(Clone, Copy, Debug)]
pub struct PerceptronHistory {
    ghr: u64,
    /// The perceptron's output; training compares its magnitude with the
    /// threshold, as the hardware carries it from prediction to update.
    output: i32,
}

impl PerceptronPredictor {
    /// Creates a new Perceptron Predictor based on configuration.
    pub fn new(config: &PerceptronConfig) -> Self {
        let table_entries = 1 << config.table_bits;
        let hist_len = config.history_length;
        let threshold = THETA_COEFF.mul_add(hist_len as f64, THETA_BIAS) as i32;
        let row_size = hist_len + 1;

        Self {
            ghr: 0,
            table: vec![0; table_entries * row_size],
            history_length: hist_len,
            table_mask: table_entries - 1,
            row_size,
            threshold,
        }
    }

    /// The weight row for `pc` under global history `ghr`.
    const fn index(&self, pc: u64, ghr: u64) -> usize {
        let pc_idx = (pc >> 2) as usize & self.table_mask;
        let hist_idx = (ghr as usize) & self.table_mask;
        pc_idx ^ hist_idx
    }

    const fn shifted(&self, ghr: u64, taken: bool) -> u64 {
        ((ghr << 1) | taken as u64) & ((1u64 << self.history_length) - 1)
    }

    /// The perceptron output for a row: the bias weight plus each history
    /// weight times the matching bit of `ghr` as +1 or -1.
    fn output(&self, row_idx: usize, ghr: u64) -> i32 {
        let base = row_idx * self.row_size;
        let mut y = self.table[base] as i32;

        for i in 0..self.history_length {
            let bit = if (ghr >> i) & 1 != 0 { 1 } else { -1 };
            y += (self.table[base + 1 + i] as i32) * bit;
        }
        y
    }
}

/// Clamps a weight value to the 8-bit signed integer range.
const fn clamp_weight(v: i32) -> i8 {
    if v > 127 {
        127
    } else if v < -128 {
        -128
    } else {
        v as i8
    }
}

impl DirectionPredictor for PerceptronPredictor {
    type History = PerceptronHistory;

    /// Predicts taken when the perceptron output (dot product) is non-negative.
    fn lookup(&self, pc: u64, _target: u64) -> (bool, PerceptronHistory) {
        let output = self.output(self.index(pc, self.ghr), self.ghr);
        (output >= 0, PerceptronHistory { ghr: self.ghr, output })
    }

    fn unconditional(&self, _pc: u64, _jump: Jump) -> PerceptronHistory {
        PerceptronHistory { ghr: self.ghr, output: 0 }
    }

    fn update_histories(&mut self, _pc: u64, taken: bool, _history: &PerceptronHistory) {
        self.ghr = self.shifted(self.ghr, taken);
    }

    fn squash(&mut self, history: &PerceptronHistory) {
        self.ghr = history.ghr;
    }

    fn correct(&mut self, _pc: u64, taken: bool, history: &PerceptronHistory) {
        self.ghr = self.shifted(history.ghr, taken);
    }

    /// Trains the weights when the prediction was wrong or its confidence
    /// (the output's magnitude) was below the training threshold.
    fn commit(&mut self, pc: u64, retired: Retired, history: &PerceptronHistory) {
        if retired.class != BranchClass::Conditional {
            return;
        }
        let ghr = history.ghr;
        let idx = self.index(pc, ghr);
        let y = history.output;
        let t = if retired.taken { 1 } else { -1 };

        if y.abs() <= self.threshold || (y >= 0) != retired.taken {
            let base = idx * self.row_size;

            let v = self.table[base] as i32 + t;
            self.table[base] = clamp_weight(v);

            for i in 0..self.history_length {
                let x = if (ghr >> i) & 1 != 0 { 1 } else { -1 };
                let w_idx = base + 1 + i;
                let v = self.table[w_idx] as i32 + t * x;
                self.table[w_idx] = clamp_weight(v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::units::bru::direction::BranchClass;

    #[test]
    fn training_uses_the_output_the_prediction_read() {
        let mut bp =
            PerceptronPredictor::new(&PerceptronConfig { history_length: 8, table_bits: 6 });
        let pc = 0x1000;
        let (_, history) = bp.lookup(pc, 0);
        let base = bp.index(pc, 0) * bp.row_size;
        bp.table[base] = 50;
        for weight in &mut bp.table[base + 1..base + bp.row_size] {
            *weight = -10;
        }

        bp.commit(
            pc,
            Retired { class: BranchClass::Conditional, taken: true, indirect_target: None },
            &history,
        );

        assert_eq!(bp.table[base], 51, "a zero output is under the threshold, so it trains");
    }
}
