//! Tag-based scoreboard for register dependency tracking.
//!
//! Maps each architectural register to the ROB tag of its latest in-flight
//! producer, or `None` if the value is in the architectural register file.
//! This enables the issue stage to do a single direct ROB lookup per source
//! operand instead of scanning the entire ROB.

use crate::isa::reg::ArchReg;
use crate::uarch::pipeline::rob::{Rob, RobTag};

/// Tag-based scoreboard: maps each architectural register to the ROB tag
/// of its latest in-flight producer, or None if the value is in the
/// architectural register file.
#[derive(Debug)]
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
    pub const fn new() -> Self {
        Self { gpr: [None; 32], fpr: [None; 32] }
    }

    /// Mark a register as having a pending writer with the given ROB tag.
    /// No-op for x0 (hardwired zero).
    pub const fn set_producer(&mut self, reg: ArchReg, tag: RobTag) {
        if let Some(slot) = self.slot_mut(reg) {
            *slot = Some(tag);
        }
    }

    /// Get the ROB tag of the latest pending writer for a register.
    /// Returns None if the register value is in the architectural register file.
    pub const fn get_producer(&self, reg: ArchReg) -> Option<RobTag> {
        match reg {
            ArchReg::Int(idx) => self.gpr[idx.as_usize()],
            ArchReg::Fp(idx) => self.fpr[idx.as_usize()],
        }
    }

    /// Clear a register's pending writer, but ONLY if the current tag matches.
    /// This prevents a committing instruction from clearing a tag set by a
    /// newer rename (WAW handling).
    pub fn clear_if_match(&mut self, reg: ArchReg, tag: RobTag) {
        if let Some(slot) = self.slot_mut(reg)
            && *slot == Some(tag)
        {
            *slot = None;
        }
    }

    /// The slot tracking `reg`'s producer; `None` for x0, which has none.
    const fn slot_mut(&mut self, reg: ArchReg) -> Option<&mut Option<RobTag>> {
        match reg {
            ArchReg::Int(idx) if idx.is_zero() => None,
            ArchReg::Int(idx) => Some(&mut self.gpr[idx.as_usize()]),
            ArchReg::Fp(idx) => Some(&mut self.fpr[idx.as_usize()]),
        }
    }

    /// Flush: clear all entries (all speculative state is gone).
    pub const fn flush(&mut self) {
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
        for entry in rob.iter() {
            if let Some(dest) = entry.ctrl.dest(entry.rd) {
                self.set_producer(dest, entry.tag);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isa::reg::RegIdx;

    #[test]
    fn test_new_all_clear() {
        let sb = Scoreboard::new();
        for i in 0u8..32 {
            assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(i))), None);
            assert_eq!(sb.get_producer(ArchReg::Fp(RegIdx::new(i))), None);
        }
    }

    #[test]
    fn test_set_and_get_producer() {
        let mut sb = Scoreboard::new();
        let tag = RobTag::new(42);
        sb.set_producer(ArchReg::Int(RegIdx::new(5)), tag);
        assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(5))), Some(tag));
        assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(6))), None);
    }

    #[test]
    fn test_x0_always_clear() {
        let mut sb = Scoreboard::new();
        sb.set_producer(ArchReg::Int(RegIdx::new(0)), RobTag::new(1));
        assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(0))), None);
    }

    #[test]
    fn test_clear_if_match() {
        let mut sb = Scoreboard::new();
        let tag = RobTag::new(10);
        sb.set_producer(ArchReg::Int(RegIdx::new(3)), tag);
        assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(3))), Some(tag));

        sb.clear_if_match(ArchReg::Int(RegIdx::new(3)), tag);
        assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(3))), None);
    }

    #[test]
    fn test_clear_mismatch_preserves() {
        let mut sb = Scoreboard::new();
        let old_tag = RobTag::new(10);
        let new_tag = RobTag::new(20);

        sb.set_producer(ArchReg::Int(RegIdx::new(3)), old_tag);
        // Newer instruction overwrites the same register
        sb.set_producer(ArchReg::Int(RegIdx::new(3)), new_tag);
        assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(3))), Some(new_tag));

        // Old instruction commits — should NOT clear because tag doesn't match
        sb.clear_if_match(ArchReg::Int(RegIdx::new(3)), old_tag);
        assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(3))), Some(new_tag));
    }

    #[test]
    fn test_flush() {
        let mut sb = Scoreboard::new();
        sb.set_producer(ArchReg::Int(RegIdx::new(1)), RobTag::new(1));
        sb.set_producer(ArchReg::Int(RegIdx::new(2)), RobTag::new(2));
        sb.set_producer(ArchReg::Fp(RegIdx::new(3)), RobTag::new(3));

        sb.flush();
        for i in 0u8..32 {
            assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(i))), None);
            assert_eq!(sb.get_producer(ArchReg::Fp(RegIdx::new(i))), None);
        }
    }

    #[test]
    fn test_fpr_independent() {
        let mut sb = Scoreboard::new();
        let gpr_tag = RobTag::new(10);
        let fpr_tag = RobTag::new(20);

        sb.set_producer(ArchReg::Int(RegIdx::new(5)), gpr_tag);
        sb.set_producer(ArchReg::Fp(RegIdx::new(5)), fpr_tag);

        assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(5))), Some(gpr_tag));
        assert_eq!(sb.get_producer(ArchReg::Fp(RegIdx::new(5))), Some(fpr_tag));

        // Clearing GPR doesn't affect FPR
        sb.clear_if_match(ArchReg::Int(RegIdx::new(5)), gpr_tag);
        assert_eq!(sb.get_producer(ArchReg::Int(RegIdx::new(5))), None);
        assert_eq!(sb.get_producer(ArchReg::Fp(RegIdx::new(5))), Some(fpr_tag));
    }
}
