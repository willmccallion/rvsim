//! Tag-based scoreboard for register dependency tracking.
//!
//! Maps each architectural register to the ROB tag of its latest in-flight
//! producer, or `None` if the value is in the architectural register file.
//! This enables the issue stage to do a single direct ROB lookup per source
//! operand instead of scanning the entire ROB.

use crate::core::pipeline::rob::{Rob, RobTag};

/// Tag-based scoreboard: maps each architectural register to the ROB tag
/// of its latest in-flight producer, or None if the value is in the
/// architectural register file.
pub struct Scoreboard {
    /// GPR scoreboard (x0 always None — hardwired zero).
    gpr: [Option<RobTag>; 32],
    /// FPR scoreboard.
    fpr: [Option<RobTag>; 32],
}

impl Default for Scoreboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Scoreboard {
    /// Create a new scoreboard with all registers clear (no pending writers).
    pub fn new() -> Self {
        Self {
            gpr: [None; 32],
            fpr: [None; 32],
        }
    }

    /// Mark a register as having a pending writer with the given ROB tag.
    /// No-op for x0 (hardwired zero).
    pub fn set_producer(&mut self, reg: usize, is_fp: bool, tag: RobTag) {
        if is_fp {
            self.fpr[reg] = Some(tag);
        } else if reg != 0 {
            self.gpr[reg] = Some(tag);
        }
    }

    /// Get the ROB tag of the latest pending writer for a register.
    /// Returns None if the register value is in the architectural register file.
    pub fn get_producer(&self, reg: usize, is_fp: bool) -> Option<RobTag> {
        if is_fp { self.fpr[reg] } else { self.gpr[reg] }
    }

    /// Clear a register's pending writer, but ONLY if the current tag matches.
    /// This prevents a committing instruction from clearing a tag set by a
    /// newer rename (WAW handling).
    pub fn clear_if_match(&mut self, reg: usize, is_fp: bool, tag: RobTag) {
        let slot = if is_fp {
            &mut self.fpr[reg]
        } else {
            &mut self.gpr[reg]
        };
        if *slot == Some(tag) {
            *slot = None;
        }
    }

    /// Flush: clear all entries (all speculative state is gone).
    pub fn flush(&mut self) {
        self.gpr = [None; 32];
        self.fpr = [None; 32];
    }

    /// Rebuild scoreboard from the remaining valid ROB entries.
    ///
    /// After a partial flush (e.g. misprediction), some ROB entries survive.
    /// We clear the scoreboard and re-mark producers from those entries,
    /// walking head-to-tail so the latest writer wins for each register.
    pub fn rebuild_from_rob(&mut self, rob: &Rob) {
        self.flush();
        rob.for_each_valid(|entry| {
            if entry.ctrl.fp_reg_write {
                self.fpr[entry.rd] = Some(entry.tag);
            } else if entry.ctrl.reg_write && entry.rd != 0 {
                self.gpr[entry.rd] = Some(entry.tag);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_all_clear() {
        let sb = Scoreboard::new();
        for i in 0..32 {
            assert_eq!(sb.get_producer(i, false), None);
            assert_eq!(sb.get_producer(i, true), None);
        }
    }

    #[test]
    fn test_set_and_get_producer() {
        let mut sb = Scoreboard::new();
        let tag = RobTag(42);
        sb.set_producer(5, false, tag);
        assert_eq!(sb.get_producer(5, false), Some(tag));
        assert_eq!(sb.get_producer(6, false), None);
    }

    #[test]
    fn test_x0_always_clear() {
        let mut sb = Scoreboard::new();
        sb.set_producer(0, false, RobTag(1));
        assert_eq!(sb.get_producer(0, false), None);
    }

    #[test]
    fn test_clear_if_match() {
        let mut sb = Scoreboard::new();
        let tag = RobTag(10);
        sb.set_producer(3, false, tag);
        assert_eq!(sb.get_producer(3, false), Some(tag));

        sb.clear_if_match(3, false, tag);
        assert_eq!(sb.get_producer(3, false), None);
    }

    #[test]
    fn test_clear_mismatch_preserves() {
        let mut sb = Scoreboard::new();
        let old_tag = RobTag(10);
        let new_tag = RobTag(20);

        sb.set_producer(3, false, old_tag);
        // Newer instruction overwrites the same register
        sb.set_producer(3, false, new_tag);
        assert_eq!(sb.get_producer(3, false), Some(new_tag));

        // Old instruction commits — should NOT clear because tag doesn't match
        sb.clear_if_match(3, false, old_tag);
        assert_eq!(sb.get_producer(3, false), Some(new_tag));
    }

    #[test]
    fn test_flush() {
        let mut sb = Scoreboard::new();
        sb.set_producer(1, false, RobTag(1));
        sb.set_producer(2, false, RobTag(2));
        sb.set_producer(3, true, RobTag(3));

        sb.flush();
        for i in 0..32 {
            assert_eq!(sb.get_producer(i, false), None);
            assert_eq!(sb.get_producer(i, true), None);
        }
    }

    #[test]
    fn test_fpr_independent() {
        let mut sb = Scoreboard::new();
        let gpr_tag = RobTag(10);
        let fpr_tag = RobTag(20);

        sb.set_producer(5, false, gpr_tag);
        sb.set_producer(5, true, fpr_tag);

        assert_eq!(sb.get_producer(5, false), Some(gpr_tag));
        assert_eq!(sb.get_producer(5, true), Some(fpr_tag));

        // Clearing GPR doesn't affect FPR
        sb.clear_if_match(5, false, gpr_tag);
        assert_eq!(sb.get_producer(5, false), None);
        assert_eq!(sb.get_producer(5, true), Some(fpr_tag));
    }
}
