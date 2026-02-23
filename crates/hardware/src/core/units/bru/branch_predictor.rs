//! Branch Predictor Interface.
//!
//! This module defines the `BranchPredictor` trait that all branch prediction
//! implementations must adhere to. It provides a common interface for
//! predicting conditional branches, indirect jumps (via BTB), and function
//! returns (via RAS).

/// Trait for branch prediction algorithms.
///
/// Defines the interface that all branch prediction implementations
/// must provide for predicting branch directions, targets, and managing
/// return address prediction.
pub trait BranchPredictor {
    /// Predicts whether a branch instruction will be taken and its target address.
    ///
    /// # Arguments
    ///
    /// * `pc` - Program counter of the branch instruction
    ///
    /// # Returns
    ///
    /// A tuple `(taken, target)` where `taken` indicates if the branch
    /// is predicted to be taken, and `target` is the predicted target
    /// address if available.
    fn predict_branch(&self, pc: u64) -> (bool, Option<u64>);

    /// Updates the branch predictor with actual branch outcome.
    ///
    /// Called after branch resolution to train the predictor with the
    /// actual taken/not-taken decision and target address.
    ///
    /// # Arguments
    ///
    /// * `pc` - Program counter of the branch instruction
    /// * `taken` - Whether the branch was actually taken
    /// * `target` - The actual target address if the branch was taken
    fn update_branch(&mut self, pc: u64, taken: bool, target: Option<u64>);

    /// Predicts the target address for a jump instruction using the BTB.
    ///
    /// # Arguments
    ///
    /// * `pc` - Program counter of the jump instruction
    ///
    /// # Returns
    ///
    /// The predicted target address if available in the BTB, `None` otherwise.
    fn predict_btb(&self, pc: u64) -> Option<u64>;

    /// Records a function call for return address prediction.
    ///
    /// Called when a call instruction (JAL/JALR with rd=ra) is executed
    /// to push the return address onto the return address stack.
    ///
    /// # Arguments
    ///
    /// * `pc` - Program counter of the call instruction
    /// * `ret_addr` - Return address (pc + instruction_size)
    /// * `target` - Target address of the call
    fn on_call(&mut self, pc: u64, ret_addr: u64, target: u64);

    /// Predicts the return address for a return instruction.
    ///
    /// # Returns
    ///
    /// The predicted return address from the return address stack,
    /// or `None` if the stack is empty.
    fn predict_return(&self) -> Option<u64>;

    /// Records a function return for return address prediction.
    ///
    /// Called when a return instruction (JALR with rd=zero, rs1=ra) is
    /// executed to pop the return address from the return address stack.
    fn on_return(&mut self);

    /// Speculatively updates the GHR with a predicted branch outcome.
    ///
    /// Called at fetch time after `predict_branch` to keep the GHR
    /// up-to-date for subsequent predictions before resolution.
    fn speculate(&mut self, _pc: u64, _taken: bool) {}

    /// Returns a snapshot of the current GHR for later repair.
    ///
    /// Called at fetch time before `speculate` so the snapshot can be
    /// carried through the pipeline and used at resolution to restore
    /// the GHR to the correct state.
    fn snapshot_history(&self) -> u64 {
        0
    }

    /// Restores the GHR to a previously captured snapshot.
    ///
    /// Called at resolution time (execute) before `update_branch` so
    /// the predictor trains on the correct history state.
    fn repair_history(&mut self, _ghr: u64) {}

    /// Returns a snapshot of the RAS pointer for speculative checkpointing.
    ///
    /// Called at fetch time before speculative push/pop so the pointer
    /// can be restored on misprediction.
    fn snapshot_ras(&self) -> usize {
        0
    }

    /// Restores the RAS pointer to a previously captured snapshot.
    ///
    /// Called on misprediction recovery to undo speculative RAS operations.
    fn restore_ras(&mut self, _ptr: usize) {}
}
