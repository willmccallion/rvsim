//! Static Branch Predictor.
//!
//! Implements a simple "Always Not Taken" prediction policy for conditional branches.
//! It utilizes a BTB for unconditional jumps and a RAS for function returns, but
//! assumes all conditional branches will fall through.

use crate::core::units::bru::{BranchPredictor, Ghr, btb::Btb, ras::Ras};

/// Static Branch Predictor structure.
#[derive(Debug)]
pub struct StaticPredictor {
    /// Branch Target Buffer for jump targets.
    btb: Btb,
    /// Return Address Stack for function returns.
    ras: Ras,
}

impl StaticPredictor {
    /// Creates a new Static Predictor.
    ///
    /// # Arguments
    ///
    /// * `btb_size` - Number of entries in the BTB.
    /// * `ras_size` - Capacity of the RAS.
    pub fn new(btb_size: usize, btb_ways: usize, ras_size: usize) -> Self {
        Self { btb: Btb::new(btb_size, btb_ways), ras: Ras::new(ras_size) }
    }
}

impl BranchPredictor for StaticPredictor {
    /// Predicts the direction and target of a branch.
    ///
    /// Always predicts conditional branches as not taken.
    fn predict_branch(&self, _pc: u64) -> (bool, Option<u64>) {
        (false, None)
    }

    /// Updates the predictor state.
    ///
    /// Only updates the BTB with the target address if a branch was taken.
    /// Does not maintain any direction history.
    fn update_branch(&mut self, pc: u64, _taken: bool, target: Option<u64>, _ghr_snapshot: &Ghr) {
        if let Some(tgt) = target {
            self.btb.update(pc, tgt);
        }
    }

    /// Predicts the target of a jump instruction using the BTB.
    fn predict_btb(&self, pc: u64) -> Option<u64> {
        self.btb.lookup(pc)
    }

    /// Handles a function call by pushing the return address to the RAS.
    fn on_call(&mut self, pc: u64, ret_addr: u64, target: u64) {
        self.ras.push(ret_addr);
        self.btb.update(pc, target);
    }

    /// Predicts the return address using the RAS.
    fn predict_return(&self) -> Option<u64> {
        self.ras.top()
    }

    /// Handles a function return by popping from the RAS.
    fn on_return(&mut self) {
        let _ = self.ras.pop();
    }

    fn snapshot_ras(&self) -> usize {
        self.ras.snapshot_ptr()
    }

    fn restore_ras(&mut self, ptr: usize) {
        self.ras.restore_ptr(ptr);
    }

    fn update_btb(&mut self, pc: u64, target: u64) {
        self.btb.update(pc, target);
    }
}
