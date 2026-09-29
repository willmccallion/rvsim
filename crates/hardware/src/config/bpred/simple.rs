//! The perceptron and tournament predictors.

use crate::config::defaults;
use serde::Deserialize;

/// Perceptron branch predictor configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct PerceptronConfig {
    /// Global history length
    #[serde(default = "PerceptronConfig::default_history")]
    pub history_length: usize,

    /// Log2 of perceptron table size
    #[serde(default = "PerceptronConfig::default_table_bits")]
    pub table_bits: usize,
}

impl Default for PerceptronConfig {
    fn default() -> Self {
        Self { history_length: Self::default_history(), table_bits: Self::default_table_bits() }
    }
}

impl PerceptronConfig {
    /// Returns the default Perceptron predictor global history length.
    const fn default_history() -> usize {
        defaults::PERCEPTRON_HISTORY
    }

    /// Returns the default Perceptron predictor table size (log2).
    const fn default_table_bits() -> usize {
        defaults::PERCEPTRON_TABLE_BITS
    }
}

/// Tournament branch predictor configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TournamentConfig {
    /// Global predictor size (log2)
    #[serde(default = "TournamentConfig::default_global")]
    pub global_size_bits: usize,

    /// Local history table size (log2)
    #[serde(default = "TournamentConfig::default_local_hist")]
    pub local_hist_bits: usize,

    /// Local prediction table size (log2)
    #[serde(default = "TournamentConfig::default_local_pred")]
    pub local_pred_bits: usize,
}

impl Default for TournamentConfig {
    fn default() -> Self {
        Self {
            global_size_bits: Self::default_global(),
            local_hist_bits: Self::default_local_hist(),
            local_pred_bits: Self::default_local_pred(),
        }
    }
}

impl TournamentConfig {
    /// Returns the default Tournament predictor global history table size (log2).
    const fn default_global() -> usize {
        defaults::TOURNAMENT_GLOBAL_BITS
    }

    /// Returns the default Tournament predictor local history table size (log2).
    const fn default_local_hist() -> usize {
        defaults::TOURNAMENT_LOCAL_HIST_BITS
    }

    /// Returns the default Tournament predictor local prediction table size (log2).
    const fn default_local_pred() -> usize {
        defaults::TOURNAMENT_LOCAL_PRED_BITS
    }
}
