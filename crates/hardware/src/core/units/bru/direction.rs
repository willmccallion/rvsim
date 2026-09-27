//! The direction predictor interface, after the hooks gem5's `BPredUnit`
//! calls on its conditional predictors.
//!
//! A prediction returns a record of what the predictor did. The branch
//! prediction unit keeps it until the instruction commits, when the
//! predictor trains on it, or is squashed, when the predictor undoes its
//! speculative history update with it.

/// Whether a control instruction's direction is predicted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchClass {
    /// A conditional branch.
    Conditional,
    /// A jump, always taken.
    Unconditional,
}

/// A committed control instruction as a predictor trains on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retired {
    /// Conditional branch or jump.
    pub class: BranchClass,
    /// Its real direction.
    pub taken: bool,
    /// Its real target when taken.
    pub target: Option<u64>,
}

/// A predictor of branch direction with speculatively updated histories.
pub trait DirectionPredictor {
    /// What one prediction left behind: the state it read and the histories
    /// before it, enough to train at commit and to undo it on a squash.
    type History;

    /// Predicts a conditional branch's direction (gem5's `lookup`).
    fn lookup(&self, pc: u64) -> (bool, Self::History);

    /// The record for a jump, whose direction is not predicted.
    fn unconditional(&self, pc: u64) -> Self::History;

    /// Shifts the speculative histories by the predicted direction.
    fn update_histories(&mut self, pc: u64, taken: bool, history: &Self::History);

    /// Undoes a squashed prediction's history update. Squashed predictions
    /// are undone youngest first.
    fn squash(&mut self, history: &Self::History);

    /// Called once after a run of [`Self::squash`] calls.
    fn squash_done(&mut self) {}

    /// Rewrites a mispredicted instruction's history update with its real
    /// direction, once everything younger has been squashed. No
    /// [`Self::squash_done`] follows, so this restores all derived state.
    fn correct(&mut self, pc: u64, taken: bool, history: &Self::History);

    /// Trains on a committed control instruction.
    fn commit(&mut self, pc: u64, retired: Retired, history: &Self::History);

    /// The predicted target of an indirect jump, when this predictor has
    /// its own indirect target predictor.
    fn indirect_target(&self, _pc: u64) -> Option<u64> {
        None
    }
}
