//! Serialize-after, as gem5's O3 rename models it: the instruction after a
//! serializing one waits in rename until the ROB has drained, so it and
//! everything behind it see the state the serializing instruction commits.
//! Rename learns the ROB is empty a cycle after commit empties it.

use crate::core::pipeline::rob::RobTag;

/// Whether rename is holding the instruction after a serializing one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Serialization {
    /// Rename runs freely.
    #[default]
    Off,
    /// The instruction after `tag` waits for the ROB to drain.
    AwaitingDrain {
        /// The serializing instruction.
        tag: RobTag,
    },
    /// The ROB drained at `cycle`.
    Drained {
        /// The cycle commit left the ROB empty.
        cycle: u64,
    },
}

impl Serialization {
    /// Holds the instruction renamed after `tag`.
    pub(super) const fn after(tag: RobTag) -> Self {
        Self::AwaitingDrain { tag }
    }

    /// Notes, after commit in cycle `now`, whether the ROB has drained.
    pub(super) const fn observe(&mut self, rob_empty: bool, now: u64) {
        if rob_empty && matches!(self, Self::AwaitingDrain { .. }) {
            *self = Self::Drained { cycle: now };
        }
    }

    /// Whether rename may take the next instruction in cycle `now`.
    #[must_use]
    pub(super) const fn admits(&mut self, now: u64) -> bool {
        match *self {
            Self::Off => true,
            Self::Drained { cycle } if now > cycle => {
                *self = Self::Off;
                true
            }
            Self::AwaitingDrain { .. } | Self::Drained { .. } => false,
        }
    }

    /// Forgets the wait when a squash removes the serializing instruction.
    pub(super) fn squash(&mut self, squashes: impl Fn(RobTag) -> bool) {
        if let Self::AwaitingDrain { tag } = *self
            && squashes(tag)
        {
            *self = Self::Off;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_rename_until_the_cycle_after_the_rob_drains() {
        let mut serialization = Serialization::after(RobTag(3));

        serialization.observe(false, 10);
        assert!(!serialization.admits(10));
        serialization.observe(true, 11);
        assert!(!serialization.admits(11));
        assert!(serialization.admits(12));
        assert_eq!(serialization, Serialization::Off);
    }

    #[test]
    fn squashing_the_serializing_instruction_releases_rename() {
        let mut serialization = Serialization::after(RobTag(3));

        serialization.squash(|tag| tag == RobTag(3));

        assert!(serialization.admits(0));
    }

    #[test]
    fn squashing_only_younger_instructions_keeps_the_wait() {
        let mut serialization = Serialization::after(RobTag(3));

        serialization.squash(|tag| tag.0 > 3);

        assert!(!serialization.admits(0));
    }
}
