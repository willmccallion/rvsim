//! The perceptron and tournament predictors.

use crate::config::defaults;
use serde::{Deserialize, Serialize};

/// Perceptron branch predictor configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct PerceptronConfig {
    /// Global history length
    pub history_length: usize,

    /// Log2 of perceptron table size
    pub table_bits: usize,
}

impl Default for PerceptronConfig {
    fn default() -> Self {
        Self {
            history_length: defaults::PERCEPTRON_HISTORY,
            table_bits: defaults::PERCEPTRON_TABLE_BITS,
        }
    }
}

/// Tournament branch predictor configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct TournamentConfig {
    /// Global predictor size (log2)
    pub global_size_bits: usize,

    /// Local history table size (log2)
    pub local_hist_bits: usize,

    /// Local prediction table size (log2)
    pub local_pred_bits: usize,
}

impl Default for TournamentConfig {
    fn default() -> Self {
        Self {
            global_size_bits: defaults::TOURNAMENT_GLOBAL_BITS,
            local_hist_bits: defaults::TOURNAMENT_LOCAL_HIST_BITS,
            local_pred_bits: defaults::TOURNAMENT_LOCAL_PRED_BITS,
        }
    }
}
