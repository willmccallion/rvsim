//! Memory Dependence Predictor Interface.
//!
//! This module defines the [`MemDepPredictor`] trait that predictor implementations
//! (e.g., store-set) implement. The trait is internal to the MDP module — the public
//! API is [`super::mem_dep_unit::MemDepUnit`].

use crate::core::pipeline::rob::RobTag;

/// Statistics collected by the memory dependence predictor.
#[derive(Default, Debug, Clone)]
pub struct MdpStats {
    /// Number of predictions that returned `Bypass`.
    pub predictions_bypass: u64,
    /// Number of predictions that returned `WaitAll`.
    pub predictions_wait_all: u64,
    /// Number of predictions that returned `WaitFor`.
    pub predictions_wait_for: u64,
    /// Number of violations (calls to `violation`).
    pub violations: u64,
}

/// Raw prediction from the predictor (SSIT/LFST lookup result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemPrediction {
    /// No known dependency.
    NoDep,
    /// Depends on this specific in-flight instruction.
    DepOn(RobTag),
}

/// Trait for memory dependence prediction algorithms.
///
/// Called by [`super::mem_dep_unit::MemDepUnit`] to query and update the
/// underlying predictor tables. Not used directly by the pipeline.
pub trait MemDepPredictor {
    /// Query SSIT+LFST for a dependency. Called once at dispatch.
    ///
    /// For stores, returns the previous store in the same set (chain predecessor)
    /// before [`register_store`](Self::register_store) overwrites the LFST entry.
    fn predict(&mut self, pc: u64, rob_tag: RobTag, is_store: bool) -> MemPrediction;

    /// Register a store in the LFST. Called AFTER `predict()` for the same store,
    /// so `predict()` sees the previous store, not itself.
    fn register_store(&mut self, store_pc: u64, rob_tag: RobTag);

    /// Train: a load at `load_pc` violated against store at `store_pc`.
    fn train(&mut self, load_pc: u64, store_pc: u64);

    /// Rebuild a single LFST entry from a surviving store after partial flush.
    /// Only overwrites if `rob_tag` is newer than the current entry.
    fn rebuild_lfst_entry(&mut self, store_pc: u64, rob_tag: RobTag);

    /// Full flush — clear LFST (SSIT persists).
    fn flush(&mut self);

    /// Partial flush — clear LFST entries newer than `keep_tag`.
    fn flush_after(&mut self, keep_tag: RobTag);

    /// Per-cycle tick (periodic SSIT clear).
    fn tick(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyPredictor;
    impl MemDepPredictor for DummyPredictor {
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

    #[test]
    fn test_mem_dep_predictor_defaults() {
        let mut predictor = DummyPredictor;
        assert_eq!(predictor.predict(0x1000, RobTag(1), false), MemPrediction::NoDep);
        predictor.train(0x1000, 0x2000);
        predictor.register_store(0x2000, RobTag(1));
        predictor.rebuild_lfst_entry(0x2000, RobTag(1));
        predictor.flush();
        predictor.flush_after(RobTag(5));
        predictor.tick();
    }

    #[test]
    fn test_prediction_eq() {
        assert_eq!(MemPrediction::NoDep, MemPrediction::NoDep);
        assert_eq!(MemPrediction::DepOn(RobTag(3)), MemPrediction::DepOn(RobTag(3)));
        assert_ne!(MemPrediction::NoDep, MemPrediction::DepOn(RobTag(1)));
        assert_ne!(MemPrediction::DepOn(RobTag(1)), MemPrediction::DepOn(RobTag(2)));
    }
}
