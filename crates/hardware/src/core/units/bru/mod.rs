//! Branch prediction unit (BRU) implementations.
//!
//! This module contains various branch prediction algorithms including
//! static prediction, gshare, perceptron, TAGE, tournament predictors,
//! branch target buffer (BTB), and return address stack (RAS).

pub use self::branch_predictor::BranchPredictor;

/// Branch predictor trait and common functionality.
pub mod branch_predictor;

/// Branch Target Buffer for storing predicted branch targets.
pub mod btb;

/// Global history branch predictor (gshare algorithm).
pub mod gshare;

/// Perceptron-based neural branch predictor.
pub mod perceptron;

/// Return Address Stack for predicting return addresses.
pub mod ras;

/// Static branch predictor (always not-taken).
pub mod static_bp;

/// Tagged Geometric History Length branch predictor.
pub mod tage;

/// Tournament branch predictor (combines local and global predictors).
pub mod tournament;

use self::{
    gshare::GSharePredictor, perceptron::PerceptronPredictor, static_bp::StaticPredictor,
    tage::TagePredictor, tournament::TournamentPredictor,
};
use crate::config::{BranchPredictor as BpType, Config};

/// Enum wrapper for static dispatch of Branch Predictors.
/// This avoids vtable lookups in the critical fetch loop.
pub enum BranchPredictorWrapper {
    Static(StaticPredictor),
    GShare(GSharePredictor),
    Tournament(TournamentPredictor),
    Tage(TagePredictor),
    Perceptron(PerceptronPredictor),
}

impl BranchPredictorWrapper {
    /// Creates a new branch predictor wrapper based on configuration.
    ///
    /// Selects the appropriate branch prediction algorithm and initializes
    /// it with the configured BTB and RAS sizes.
    pub fn new(config: &Config) -> Self {
        let btb_size = config.pipeline.btb_size;
        let ras_size = config.pipeline.ras_size;

        match config.pipeline.branch_predictor {
            BpType::Static => Self::Static(StaticPredictor::new(btb_size, ras_size)),
            BpType::GShare => Self::GShare(GSharePredictor::new(btb_size, ras_size)),
            BpType::Tournament => Self::Tournament(TournamentPredictor::new(
                &config.pipeline.tournament,
                btb_size,
                ras_size,
            )),
            BpType::Tage => Self::Tage(TagePredictor::new(
                &config.pipeline.tage,
                btb_size,
                ras_size,
            )),
            BpType::Perceptron => Self::Perceptron(PerceptronPredictor::new(
                &config.pipeline.perceptron,
                btb_size,
                ras_size,
            )),
        }
    }
}

impl BranchPredictor for BranchPredictorWrapper {
    /// Predicts whether a branch at the given PC will be taken and its target.
    ///
    /// Returns a tuple of (taken, target_opt) where target_opt is Some(target)
    /// if the branch is predicted taken, otherwise None.
    #[inline(always)]
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>) {
        match self {
            Self::Static(bp) => bp.predict_branch(pc),
            Self::GShare(bp) => bp.predict_branch(pc),
            Self::Tournament(bp) => bp.predict_branch(pc),
            Self::Tage(bp) => bp.predict_branch(pc),
            Self::Perceptron(bp) => bp.predict_branch(pc),
        }
    }

    /// Updates the branch predictor with the actual outcome of a branch.
    ///
    /// Called after branch resolution to train the predictor and update
    /// internal state based on whether the branch was taken and its target.
    #[inline(always)]
    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>) {
        match self {
            Self::Static(bp) => bp.update_branch(pc, taken, target),
            Self::GShare(bp) => bp.update_branch(pc, taken, target),
            Self::Tournament(bp) => bp.update_branch(pc, taken, target),
            Self::Tage(bp) => bp.update_branch(pc, taken, target),
            Self::Perceptron(bp) => bp.update_branch(pc, taken, target),
        }
    }

    /// Predicts the target address for a branch at the given PC using the BTB.
    ///
    /// Returns Some(target) if a prediction exists in the BTB, otherwise None.
    #[inline(always)]
    fn predict_btb(&self, pc: u64) -> Option<u64> {
        match self {
            Self::Static(bp) => bp.predict_btb(pc),
            Self::GShare(bp) => bp.predict_btb(pc),
            Self::Tournament(bp) => bp.predict_btb(pc),
            Self::Tage(bp) => bp.predict_btb(pc),
            Self::Perceptron(bp) => bp.predict_btb(pc),
        }
    }

    /// Records a function call for return address prediction.
    ///
    /// Pushes the return address onto the RAS when a call instruction is executed.
    #[inline(always)]
    fn on_call(&mut self, pc: u64, ret_addr: u64, target: u64) {
        match self {
            Self::Static(bp) => bp.on_call(pc, ret_addr, target),
            Self::GShare(bp) => bp.on_call(pc, ret_addr, target),
            Self::Tournament(bp) => bp.on_call(pc, ret_addr, target),
            Self::Tage(bp) => bp.on_call(pc, ret_addr, target),
            Self::Perceptron(bp) => bp.on_call(pc, ret_addr, target),
        }
    }

    /// Predicts the return address for a return instruction.
    ///
    /// Pops the top address from the RAS, which should be the return address
    /// from the most recent call.
    #[inline(always)]
    fn predict_return(&self) -> Option<u64> {
        match self {
            Self::Static(bp) => bp.predict_return(),
            Self::GShare(bp) => bp.predict_return(),
            Self::Tournament(bp) => bp.predict_return(),
            Self::Tage(bp) => bp.predict_return(),
            Self::Perceptron(bp) => bp.predict_return(),
        }
    }

    /// Records a function return for RAS management.
    ///
    /// Pops the return address from the RAS when a return instruction is executed.
    #[inline(always)]
    fn on_return(&mut self) {
        match self {
            Self::Static(bp) => bp.on_return(),
            Self::GShare(bp) => bp.on_return(),
            Self::Tournament(bp) => bp.on_return(),
            Self::Tage(bp) => bp.on_return(),
            Self::Perceptron(bp) => bp.on_return(),
        }
    }
}
