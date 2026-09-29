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
use crate::config::{BranchPredictorKind, Config};

/// The configured prediction unit, dispatched statically so the fetch
/// loop makes no virtual calls.
#[derive(Debug)]
pub enum BranchPredictor {
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
            BranchPredictor::Static($unit) => $call,
            BranchPredictor::GShare($unit) => $call,
            BranchPredictor::Tournament($unit) => $call,
            BranchPredictor::Tage($unit) => $call,
            BranchPredictor::Perceptron($unit) => $call,
            BranchPredictor::ScLTage($unit) => $call,
        }
    };
}

impl BranchPredictor {
    /// Creates the unit for the configured predictor, with the configured
    /// BTB and RAS.
    pub fn new(config: &Config) -> Self {
        let pipeline = &config.pipeline;

        match pipeline.branch_predictor {
            BranchPredictorKind::Static => Self::Static(unit_for(config, StaticPredictor::new())),
            BranchPredictorKind::GShare => Self::GShare(unit_for(config, GSharePredictor::new())),
            BranchPredictorKind::Tournament => {
                Self::Tournament(unit_for(config, TournamentPredictor::new(&pipeline.tournament)))
            }
            BranchPredictorKind::Tage => {
                Self::Tage(Box::new(unit_for(config, TagePredictor::new(&pipeline.tage))))
            }
            BranchPredictorKind::Perceptron => {
                Self::Perceptron(unit_for(config, PerceptronPredictor::new(&pipeline.perceptron)))
            }
            BranchPredictorKind::ScLTage => Self::ScLTage(Box::new(unit_for(
                config,
                ScLTagePredictor::new(
                    &pipeline.tage,
                    &pipeline.sc,
                    &pipeline.ittage,
                    &pipeline.loop_predictor,
                ),
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

    /// What the BTB holds for the control instruction at `pc`.
    #[must_use]
    pub fn btb_lookup(&self, pc: u64) -> Option<btb::BtbHit> {
        dispatch!(self, unit => unit.btb_lookup(pc))
    }

    /// True when fetch predicted instruction `seq`.
    #[must_use]
    pub fn is_predicted(&self, seq: InstSeq) -> bool {
        dispatch!(self, unit => unit.is_predicted(seq))
    }

    /// See [`unit::BranchPredUnit::discover`].
    pub fn discover(&mut self, seq: InstSeq, pc: u64, inst: ControlInst) -> (Option<u64>, bool) {
        dispatch!(self, unit => unit.discover(seq, pc, inst))
    }

    /// See [`unit::BranchPredUnit::correct_target`].
    pub fn correct_target(&mut self, seq: InstSeq, target: u64) {
        dispatch!(self, unit => unit.correct_target(seq, target));
    }

    /// See [`unit::BranchPredUnit::forget`].
    pub fn forget(&mut self, seq: InstSeq, pc: u64) {
        dispatch!(self, unit => unit.forget(seq, pc));
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
}
