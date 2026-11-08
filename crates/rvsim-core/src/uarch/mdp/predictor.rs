//! What a memory dependence predictor reports: its predictions and
//! statistics. The public API is [`super::unit::MemDepUnit`].

use crate::uarch::pipeline::rob::RobTag;

/// Statistics collected by the memory dependence predictor.
#[derive(Default, Debug, Clone)]
pub struct MdpStats {
    /// Loads predicted independent of every older store.
    pub predictions_bypass: u64,
    /// Loads made to wait for every older store's address: all of them
    /// under the blind predictor, and LRs and AMOs under store sets.
    pub predictions_wait_all: u64,
    /// Loads and stores made to wait for one older store in their set.
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
