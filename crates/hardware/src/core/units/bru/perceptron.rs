//! Perceptron Branch Predictor.
//!
//! Uses a single-layer perceptron neural network to predict branch direction.
//! Instead of saturating counters, it uses a table of weight vectors. The
//! prediction is the dot product of the weights and the history vector.

use super::{BranchPredictor, btb::Btb, ras::Ras};
use crate::config::PerceptronConfig;

/// Coefficient used to calculate the training threshold.
const THETA_COEFF: f64 = 1.93;
/// Bias used to calculate the training threshold.
const THETA_BIAS: f64 = 14.0;

/// Perceptron Predictor structure.
pub struct PerceptronPredictor {
    /// Global History Register.
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
    /// Branch Target Buffer.
    btb: Btb,
    /// Return Address Stack.
    ras: Ras,
}

impl PerceptronPredictor {
    /// Creates a new Perceptron Predictor based on configuration.
    pub fn new(config: &PerceptronConfig, btb_size: usize, ras_size: usize) -> Self {
        let table_entries = 1 << config.table_bits;
        let hist_len = config.history_length;
        let threshold = (THETA_COEFF * (hist_len as f64) + THETA_BIAS) as i32;
        let row_size = hist_len + 1;

        Self {
            ghr: 0,
            table: vec![0; table_entries * row_size],
            history_length: hist_len,
            table_mask: table_entries - 1,
            row_size,
            threshold,
            btb: Btb::new(btb_size),
            ras: Ras::new(ras_size),
        }
    }

    /// Calculates the index into the weight table using PC and GHR hash.
    fn index(&self, pc: u64) -> usize {
        let pc_idx = (pc >> 2) as usize & self.table_mask;
        let hist_idx = (self.ghr as usize) & self.table_mask;
        pc_idx ^ hist_idx
    }

    /// Computes the perceptron output (dot product) for a given row.
    ///
    /// Sums the bias weight and the product of history bits and weights.
    fn output(&self, row_idx: usize) -> i32 {
        let base = row_idx * self.row_size;
        let mut y = self.table[base] as i32;

        for i in 0..self.history_length {
            let bit = if (self.ghr >> i) & 1 != 0 { 1 } else { -1 };
            y += (self.table[base + 1 + i] as i32) * bit;
        }
        y
    }
}

/// Clamps a weight value to the 8-bit signed integer range.
fn clamp_weight(v: i32) -> i8 {
    if v > 127 {
        127
    } else if v < -128 {
        -128
    } else {
        v as i8
    }
}

impl BranchPredictor for PerceptronPredictor {
    /// Predicts branch direction and target.
    ///
    /// Predicts taken if the perceptron output (dot product) is non-negative.
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        let idx = self.index(pc);
        let y = self.output(idx);
        let taken = y >= 0;
        if taken {
            (true, self.btb.lookup(pc))
        } else {
            (false, None)
        }
    }

    /// Updates the predictor weights based on the actual outcome.
    ///
    /// Trains the perceptron if a misprediction occurred or if the confidence
    /// (magnitude of the output) was below the training threshold.
    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>) {
        let idx = self.index(pc);
        let y = self.output(idx);
        let t = if taken { 1 } else { -1 };

        if y.abs() <= self.threshold || (y >= 0) != taken {
            let base = idx * self.row_size;

            let v = self.table[base] as i32 + t;
            self.table[base] = clamp_weight(v);

            for i in 0..self.history_length {
                let x = if (self.ghr >> i) & 1 != 0 { 1 } else { -1 };
                let w_idx = base + 1 + i;
                let v = self.table[w_idx] as i32 + t * x;
                self.table[w_idx] = clamp_weight(v);
            }
        }

        self.ghr =
            ((self.ghr << 1) | if taken { 1 } else { 0 }) & ((1u64 << self.history_length) - 1);

        if let Some(tgt) = target {
            self.btb.update(pc, tgt);
        }
    }

    /// Predicts the target of a jump instruction using the BTB.
    fn predict_btb(&self, pc: u64) -> Option<u64> {
        self.btb.lookup(pc)
    }

    /// Handles a function call by pushing the return address to the RAS.
    fn on_call(&mut self, pc: u64, ret_addr: u64, target: u64) {
        self.ras.push(ret_addr);
        self.btb.update(pc, target);
    }

    /// Predicts the return address using the RAS.
    fn predict_return(&self) -> Option<u64> {
        self.ras.top()
    }

    /// Handles a function return by popping from the RAS.
    fn on_return(&mut self) {
        self.ras.pop();
    }
}
