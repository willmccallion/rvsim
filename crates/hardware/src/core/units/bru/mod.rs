//! Branch prediction unit (BRU) implementations.
//!
//! This module contains various branch prediction algorithms including
//! static prediction, gshare, perceptron, TAGE, tournament, and SC-L-TAGE
//! predictors, along with branch target buffer (BTB) and return address
//! stack (RAS), composed into a prediction unit.

pub use self::direction::{BranchClass, DirectionPredictor, Retired};
pub use self::ghr::Ghr;
pub use self::unit::{BranchPredUnit, ControlInst};

pub mod btb;

pub mod components;

pub mod direction;

pub mod ghr;

pub mod predictors;

pub mod ras;

pub mod unit;

use self::predictors::{
    gshare::GSharePredictor, perceptron::PerceptronPredictor, sc_l_tage::ScLTagePredictor,
    static_bp::StaticPredictor, tage::TagePredictor, tournament::TournamentPredictor,
};
use crate::common::InstSeq;
use crate::config::{BranchPredictor as BpType, Config};

/// The configured prediction unit, dispatched statically so the fetch
/// loop makes no virtual calls.
#[derive(Debug)]
pub enum BranchPredictorWrapper {
    /// Static (always not-taken) predictor.
    Static(BranchPredUnit<StaticPredictor>),
    /// Global history (gshare) predictor.
    GShare(BranchPredUnit<GSharePredictor>),
    /// Tournament predictor combining local and global histories.
    Tournament(BranchPredUnit<TournamentPredictor>),
    /// TAGE predictor with geometric history lengths.
    Tage(Box<BranchPredUnit<TagePredictor>>),
    /// Perceptron-based neural predictor.
    Perceptron(BranchPredUnit<PerceptronPredictor>),
    /// SC-L-TAGE + ITTAGE composed predictor.
    ScLTage(Box<BranchPredUnit<ScLTagePredictor>>),
}

/// A prediction unit around `direction` with the configured BTB and RAS.
fn unit_for<P: DirectionPredictor>(config: &Config, direction: P) -> BranchPredUnit<P> {
    let pipeline = &config.pipeline;
    BranchPredUnit::new(direction, pipeline.btb_size, pipeline.btb_ways, pipeline.ras_size)
}

/// Runs `$call` on whichever unit `$wrapper` holds.
macro_rules! dispatch {
    ($wrapper:expr, $unit:ident => $call:expr) => {
        match $wrapper {
            BranchPredictorWrapper::Static($unit) => $call,
            BranchPredictorWrapper::GShare($unit) => $call,
            BranchPredictorWrapper::Tournament($unit) => $call,
            BranchPredictorWrapper::Tage($unit) => $call,
            BranchPredictorWrapper::Perceptron($unit) => $call,
            BranchPredictorWrapper::ScLTage($unit) => $call,
        }
    };
}

impl BranchPredictorWrapper {
    /// Creates the unit for the configured predictor, with the configured
    /// BTB and RAS.
    pub fn new(config: &Config) -> Self {
        let pipeline = &config.pipeline;

        match pipeline.branch_predictor {
            BpType::Static => Self::Static(unit_for(config, StaticPredictor::new())),
            BpType::GShare => Self::GShare(unit_for(config, GSharePredictor::new())),
            BpType::Tournament => {
                Self::Tournament(unit_for(config, TournamentPredictor::new(&pipeline.tournament)))
            }
            BpType::Tage => {
                Self::Tage(Box::new(unit_for(config, TagePredictor::new(&pipeline.tage))))
            }
            BpType::Perceptron => {
                Self::Perceptron(unit_for(config, PerceptronPredictor::new(&pipeline.perceptron)))
            }
            BpType::ScLTage => Self::ScLTage(Box::new(unit_for(
                config,
                ScLTagePredictor::new(&pipeline.tage, &pipeline.sc, &pipeline.ittage),
            ))),
        }
    }

    /// See [`BranchPredUnit::predict`].
    #[inline(always)]
    pub fn predict(&mut self, seq: InstSeq, pc: u64, inst: ControlInst) -> Option<u64> {
        dispatch!(self, unit => unit.predict(seq, pc, inst))
    }

    /// See [`BranchPredUnit::squash_after`].
    pub fn squash_after(&mut self, keep: InstSeq) {
        dispatch!(self, unit => unit.squash_after(keep));
    }

    /// See [`BranchPredUnit::squash_all`].
    pub fn squash_all(&mut self) {
        dispatch!(self, unit => unit.squash_all());
    }

    /// See [`BranchPredUnit::mispredict`].
    pub fn mispredict(&mut self, seq: InstSeq, taken: bool, target: u64) {
        dispatch!(self, unit => unit.mispredict(seq, taken, target));
    }

    /// See [`BranchPredUnit::commit`].
    pub fn commit(&mut self, done: InstSeq) {
        dispatch!(self, unit => unit.commit(done));
    }

    /// See [`BranchPredUnit::update_btb`].
    pub fn update_btb(&mut self, pc: u64, target: u64) {
        dispatch!(self, unit => unit.update_btb(pc, target));
    }
}
