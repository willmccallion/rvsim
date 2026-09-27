//! A squash an instruction asks for when it executes, taken by the
//! pipeline a fixed number of cycles later.
//!
//! Execute never redirects fetch itself. A mispredicted branch, a CSR
//! write, a fault or a memory-ordering violation produces a [`Redirect`];
//! the engine files it as a [`PendingSquash`] against the instruction's
//! ROB tag and applies it once `pipeline.redirect_latency` cycles have
//! passed, the way gem5's squash travels from execute through commit to
//! fetch. Until then commit retires nothing younger than the instruction.

use crate::common::InstSeq;
use crate::core::pipeline::rob::RobTag;
use crate::core::units::bru::BranchPredictorWrapper;

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

impl SquashCause {
    /// True when the squash replays the instruction after the kept one from
    /// the PC it was fetched at, rather than redirecting to where the kept
    /// instruction itself says execution goes next.
    #[must_use]
    pub const fn replays(self) -> bool {
        matches!(self, Self::MemoryOrder | Self::Coherence)
    }
}

/// A control instruction that resolved against its prediction: the
/// predictor squashes what it predicted after it and rewrites its own
/// history update with the real outcome.
#[derive(Clone, Copy, Debug)]
pub struct BranchRepair {
    /// The instruction.
    pub seq: InstSeq,
    /// Its real direction (always taken for a jump).
    pub taken: bool,
    /// Where it goes when taken.
    pub target: u64,
}

impl BranchRepair {
    /// Repairs the predictor for this misprediction.
    pub fn apply(&self, predictor: &mut BranchPredictorWrapper) {
        predictor.mispredict(self.seq, self.taken, self.target);
    }
}

/// What execute decided about the instructions younger than one it ran.
///
/// They are dropped and fetch resumes at `target`. An instruction that
/// only needs its successors refetched (an xRET, a fault, a CSR write)
/// targets its own successor; what commit then does with it (a trap, a
/// return) redirects fetch again.
#[derive(Clone, Copy, Debug)]
pub struct Redirect {
    /// Where fetch resumes.
    pub target: u64,
    /// Why.
    pub cause: SquashCause,
    /// Predictor repair for a mispredicted branch.
    pub repair: Option<BranchRepair>,
}

impl Redirect {
    /// Resume fetch at `target` once the younger instructions are gone.
    #[must_use]
    pub const fn to(target: u64, cause: SquashCause) -> Self {
        Self { target, cause, repair: None }
    }

    /// A mispredicted branch: resume at its real target and repair the
    /// predictor.
    #[must_use]
    pub const fn mispredict(target: u64, repair: BranchRepair) -> Self {
        Self { target, cause: SquashCause::Branch, repair: Some(repair) }
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

    /// True when this squash should be taken instead of `other`: it removes
    /// more of the window, or it removes the same and redirects from the kept
    /// instruction while `other` only replays the one after it, which was
    /// fetched down a path the kept instruction may be about to correct.
    #[must_use]
    pub const fn takes_precedence_over(&self, other: &Self) -> bool {
        if self.is_older_than(other) {
            return true;
        }
        let same_window = match (self.keep_tag, other.keep_tag) {
            (Some(mine), Some(theirs)) => mine.0 == theirs.0,
            (None, None) => true,
            _ => false,
        };
        same_window && other.redirect.cause.replays() && !self.redirect.cause.replays()
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
            redirect: Redirect::to(0, SquashCause::System),
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

    fn keeping(keep_tag: u32, cause: SquashCause) -> PendingSquash {
        PendingSquash {
            keep_tag: Some(RobTag(keep_tag)),
            redirect: Redirect::to(0, cause),
            apply_at: 0,
        }
    }

    #[test]
    fn a_redirect_from_the_kept_instruction_beats_a_replay_of_the_next() {
        let violation = keeping(5, SquashCause::MemoryOrder);
        let mispredict = keeping(5, SquashCause::Branch);

        assert!(mispredict.takes_precedence_over(&violation));
        assert!(!violation.takes_precedence_over(&mispredict));
    }

    #[test]
    fn a_squash_that_removes_more_takes_precedence_whatever_its_cause() {
        let older_violation = keeping(3, SquashCause::Coherence);
        let younger_mispredict = keeping(7, SquashCause::Branch);

        assert!(older_violation.takes_precedence_over(&younger_mispredict));
        assert!(!younger_mispredict.takes_precedence_over(&older_violation));
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
