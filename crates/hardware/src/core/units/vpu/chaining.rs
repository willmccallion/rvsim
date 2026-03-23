//! Vector chaining model (element-group granularity).
//!
//! Tracks pending vector results and fires chaining wakeups at
//! `first_group_ready` (enabling dependent vector instructions to start
//! executing) and marks ROB complete at `full_complete`.
//!
//! Based on Saturn's "augmented element-group-granularity scoreboarding".

use crate::core::pipeline::rob::RobTag;
use crate::core::units::vpu::types::VecPhysReg;

/// A pending vector execution result, tracking chaining and completion timing.
#[derive(Debug, Clone)]
pub struct VecPendingResult {
    /// ROB tag of the vector instruction.
    pub rob_tag: RobTag,
    /// Physical destination registers for the LMUL group.
    pub vd_phys: [VecPhysReg; 8],
    /// Number of destination registers in the LMUL group (1/2/4/8).
    pub vd_count: u8,
    /// Cycle at which the first element-group result is ready (chaining wakeup).
    pub first_group_ready: u64,
    /// Cycle at which all element-groups are complete (ROB complete).
    pub full_complete: u64,
    /// Whether the chaining wakeup has already been fired.
    pub wakeup_fired: bool,
}

impl VecPendingResult {
    /// Returns true if the chaining wakeup should fire at cycle `now`.
    #[inline]
    pub const fn should_wakeup(&self, now: u64) -> bool {
        !self.wakeup_fired && now >= self.first_group_ready
    }

    /// Returns true if the instruction is fully complete at cycle `now`.
    #[inline]
    pub const fn is_complete(&self, now: u64) -> bool {
        now >= self.full_complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pending(first: u64, full: u64) -> VecPendingResult {
        VecPendingResult {
            rob_tag: RobTag(1),
            vd_phys: [VecPhysReg::ZERO; 8],
            vd_count: 1,
            first_group_ready: first,
            full_complete: full,
            wakeup_fired: false,
        }
    }

    #[test]
    fn test_wakeup_timing() {
        let mut pr = make_pending(10, 15);
        assert!(!pr.should_wakeup(9));
        assert!(pr.should_wakeup(10));
        assert!(pr.should_wakeup(11));

        pr.wakeup_fired = true;
        assert!(!pr.should_wakeup(11));
    }

    #[test]
    fn test_completion_timing() {
        let pr = make_pending(10, 15);
        assert!(!pr.is_complete(14));
        assert!(pr.is_complete(15));
        assert!(pr.is_complete(16));
    }

    #[test]
    fn test_no_chaining() {
        // When chaining disabled: first_group_ready == full_complete
        let pr = make_pending(15, 15);
        assert!(!pr.should_wakeup(14));
        assert!(pr.should_wakeup(15));
        assert!(pr.is_complete(15));
    }
}
