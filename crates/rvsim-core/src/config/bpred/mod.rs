//! Branch predictor configuration.

use serde::Deserialize;

/// Branch prediction algorithm types.
///
/// Specifies the branch prediction algorithm used to predict
/// branch directions and targets for improved pipeline performance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BranchPredictorKind {
    /// Static branch predictor (always predict not-taken).
    ///
    /// Simple predictor that always predicts branches as not-taken.
    #[default]
    Static,
    /// Global history branch predictor (gshare).
    ///
    /// Uses global branch history to index a pattern history table.
    GShare,
    /// Perceptron-based neural branch predictor.
    ///
    /// Uses a neural network (perceptron) to learn branch patterns.
    Perceptron,
    /// Tagged Geometric History Length predictor.
    ///
    /// Advanced predictor using multiple history lengths with tags.
    #[serde(alias = "TAGE")]
    Tage,
    /// Tournament predictor combining local and global predictors.
    ///
    /// Selects between local and global predictors based on performance.
    Tournament,
    /// SC-L-TAGE + ITTAGE composed predictor.
    ///
    /// Combines TAGE, Loop, Statistical Corrector, and Indirect Target TAGE.
    #[serde(alias = "SC-L-TAGE")]
    ScLTage,
}

mod ittage;
mod loop_predictor;
mod sc;
mod simple;
mod tage;

pub use ittage::IttageConfig;
pub use loop_predictor::LoopConfig;
pub use sc::{
    GehlConfig, LocalGehlConfig, MAX_GEHL_TABLES, MAX_LOCAL_HISTORIES, ScConfig, ScConfigError,
};
pub use simple::{PerceptronConfig, TournamentConfig};
pub use tage::{
    MAX_TAGE_BANKS, MAX_TAGE_HISTORY, TageAllocation, TageBanking, TageConfig, TageHashing,
    TageHistoryMode, TageUpdate,
};
