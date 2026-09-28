//! Static Branch Predictor.
//!
//! Predicts every conditional branch not taken. Jumps still take their
//! targets from the BTB and the return address stack in the prediction unit.

use crate::core::units::bru::direction::{DirectionPredictor, Jump, Retired};

/// Static Branch Predictor structure.
#[derive(Debug, Default)]
pub struct StaticPredictor;

impl StaticPredictor {
    /// Creates a new Static Predictor.
    pub const fn new() -> Self {
        Self
    }
}

impl DirectionPredictor for StaticPredictor {
    type History = ();

    fn lookup(&self, _pc: u64, _target: u64) -> (bool, ()) {
        (false, ())
    }

    fn unconditional(&self, _pc: u64, _jump: Jump) {}

    fn update_histories(&mut self, _pc: u64, _taken: bool, _history: &()) {}

    fn squash(&mut self, _history: &()) {}

    fn correct(&mut self, _pc: u64, _taken: bool, _history: &()) {}

    fn commit(&mut self, _pc: u64, _retired: Retired, _history: &()) {}
}
