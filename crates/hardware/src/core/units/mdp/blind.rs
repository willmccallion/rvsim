//! Blind (conservative) memory dependence predictor.
//!
//! Always returns [`NoDep`](super::mem_dep_predictor::MemPrediction::NoDep).
//! The [`MemDepUnit`](super::mem_dep_unit::MemDepUnit) converts this to `WaitAll`
//! for loads, preserving baseline conservative behavior.

use crate::core::pipeline::rob::RobTag;

use super::mem_dep_predictor::{MemDepPredictor, MemPrediction};

/// Blind memory dependence predictor (always conservative).
///
/// This is the default predictor. The `MemDepUnit` wrapping layer converts
/// `NoDep` into `WaitAll` for loads, matching the pre-MDP behavior.
///
/// Not constructed directly — the `MemDepUnit` handles Blind mode internally
/// via `PredictorKind::Blind`. This struct exists for trait completeness and tests.
#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct BlindPredictor;

#[cfg_attr(not(test), allow(dead_code))]
impl BlindPredictor {
    /// Creates a new blind predictor.
    pub const fn new() -> Self {
        Self
    }
}

impl MemDepPredictor for BlindPredictor {
    #[inline(always)]
    fn predict(&mut self, _pc: u64, _rob_tag: RobTag, _is_store: bool) -> MemPrediction {
        MemPrediction::NoDep
    }

    fn register_store(&mut self, _store_pc: u64, _rob_tag: RobTag) {}

    fn train(&mut self, _load_pc: u64, _store_pc: u64) {}

    fn rebuild_lfst_entry(&mut self, _store_pc: u64, _rob_tag: RobTag) {}

    fn flush(&mut self) {}

    fn flush_after(&mut self, _keep_tag: RobTag) {}

    fn tick(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blind_always_no_dep() {
        let mut bp = BlindPredictor::new();
        assert_eq!(bp.predict(0x0, RobTag(1), false), MemPrediction::NoDep);
        assert_eq!(bp.predict(0x1000, RobTag(2), false), MemPrediction::NoDep);
        assert_eq!(bp.predict(0x1000, RobTag(3), true), MemPrediction::NoDep);
    }

    #[test]
    fn test_blind_train_is_noop() {
        let mut bp = BlindPredictor::new();
        bp.train(0x1000, 0x2000);
        assert_eq!(bp.predict(0x1000, RobTag(1), false), MemPrediction::NoDep);
    }
}
