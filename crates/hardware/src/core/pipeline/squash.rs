//! A squash an instruction asks for when it executes, taken by the
//! pipeline a fixed number of cycles later.
//!
//! Execute never redirects fetch itself. A mispredicted branch, a CSR
//! write, a fault or a memory-ordering violation produces a [`Redirect`];
//! the engine files it as a [`PendingSquash`] against the instruction's
//! ROB tag and applies it once `pipeline.redirect_latency` cycles have
//! passed, the way gem5's squash travels from execute through commit to
//! fetch. Until then commit retires nothing younger than the instruction.

use crate::core::pipeline::rob::RobTag;
use crate::core::units::bru::{BranchPredictor, Ghr, RasSnapshot};

/// Why an instruction squashed what followed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SquashCause {
    /// A branch or jump resolved against its prediction.
    Branch,
    /// A system instruction, CSR write or execute-stage fault.
    System,
    /// A load read stale data past an older store.
    MemoryOrder,
    /// A load read a value another hart has since overwritten.
    Coherence,
}

/// The predictor state a mispredicted branch restores when its squash is
/// taken: the global history as fetched, with the real outcome pushed,
/// and the return-address stack as it was before the branch.
#[derive(Clone, Copy, Debug)]
pub struct BranchRepair {
    /// The branch.
    pub pc: u64,
    /// Its real direction (always taken for a jump).
    pub taken: bool,
    /// Global history captured when the branch was fetched.
    pub ghr: Ghr,
    /// Return-address stack captured when the branch was fetched.
    pub ras: RasSnapshot,
}

impl BranchRepair {
    /// Restores the predictor to this branch's fetch-time state and
    /// records its real outcome.
    pub fn apply(&self, predictor: &mut impl BranchPredictor) {
        predictor.repair_history(&self.ghr);
        predictor.speculate(self.pc, self.taken);
        predictor.restore_ras(self.ras);
    }
}

/// What execute decided about the instructions younger than one it ran.
#[derive(Clone, Copy, Debug)]
pub struct Redirect {
    /// Where fetch resumes. `None` squashes the younger instructions but
    /// leaves the fetch PC alone; commit will redirect when the
    /// instruction retires (an xRET, a fault).
    pub target: Option<u64>,
    /// Why.
    pub cause: SquashCause,
    /// Predictor repair for a mispredicted branch.
    pub repair: Option<BranchRepair>,
}

impl Redirect {
    /// Resume fetch at `target` once the younger instructions are gone.
    #[must_use]
    pub const fn to(target: u64, cause: SquashCause) -> Self {
        Self { target: Some(target), cause, repair: None }
    }

    /// Drop the younger instructions and keep fetching where fetch is.
    #[must_use]
    pub const fn squash_younger(cause: SquashCause) -> Self {
        Self { target: None, cause, repair: None }
    }

    /// A mispredicted branch: resume at its real target and repair the
    /// predictor.
    #[must_use]
    pub const fn mispredict(target: u64, repair: BranchRepair) -> Self {
        Self { target: Some(target), cause: SquashCause::Branch, repair: Some(repair) }
    }
}

/// A redirect waiting for its latency to elapse.
#[derive(Clone, Copy, Debug)]
pub struct PendingSquash {
    /// The youngest instruction that survives; `None` squashes the whole
    /// window.
    pub keep_tag: Option<RobTag>,
    /// What to do once the squash is taken.
    pub redirect: Redirect,
    /// The cycle the squash is taken.
    pub apply_at: u64,
}

impl PendingSquash {
    /// True when the squash will remove `tag`, so it must not retire or
    /// touch anything outside the pipeline before then.
    #[must_use]
    pub fn squashes(&self, tag: RobTag) -> bool {
        self.keep_tag.is_none_or(|keep| tag.is_newer_than(keep))
    }

    /// True when this squash removes more of the window than `other`.
    #[must_use]
    pub const fn is_older_than(&self, other: &Self) -> bool {
        match (self.keep_tag, other.keep_tag) {
            (None, Some(_)) => true,
            (Some(mine), Some(theirs)) => mine.is_older_than(theirs),
            (None | Some(_), None) => false,
        }
    }

    /// True when the latency has elapsed at `now`.
    #[must_use]
    pub const fn is_due(&self, now: u64) -> bool {
        self.apply_at <= now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(keep_tag: Option<u32>) -> PendingSquash {
        PendingSquash {
            keep_tag: keep_tag.map(RobTag),
            redirect: Redirect::squash_younger(SquashCause::System),
            apply_at: 0,
        }
    }

    #[test]
    fn squashes_everything_younger_than_the_kept_tag() {
        let squash = pending(Some(5));
        assert!(!squash.squashes(RobTag(4)));
        assert!(!squash.squashes(RobTag(5)));
        assert!(squash.squashes(RobTag(6)));
    }

    #[test]
    fn a_squash_without_a_kept_tag_squashes_the_whole_window() {
        assert!(pending(None).squashes(RobTag(0)));
    }

    #[test]
    fn the_squash_keeping_less_is_the_older_one() {
        assert!(pending(Some(3)).is_older_than(&pending(Some(7))));
        assert!(!pending(Some(7)).is_older_than(&pending(Some(3))));
        assert!(pending(None).is_older_than(&pending(Some(0))));
        assert!(!pending(Some(0)).is_older_than(&pending(None)));
        assert!(!pending(Some(3)).is_older_than(&pending(Some(3))));
    }
}
