//! Speculative rename map: architectural register → current physical register.
//!
//! Tracks the current mapping from architectural register names to physical
//! register numbers. Maintained speculatively; the committed_rename_map
//! in O3Engine tracks the last committed state for flush recovery.

use crate::common::RegIdx;

use super::prf::PhysReg;

/// Speculative rename map for GPRs and FPRs.
#[derive(Clone, Debug)]
pub struct RenameMap {
    /// GPR rename map: gpr[i] = current physical reg for `x_i`
    gpr: [PhysReg; 32],
    /// FPR rename map: fpr[i] = current physical reg for `f_i`
    fpr: [PhysReg; 32],
}

impl RenameMap {
    /// Create a new rename map with identity mapping.
    /// GPR i → PhysReg(i), FPR i → PhysReg(32 + i).
    pub fn new() -> Self {
        let mut gpr = [PhysReg(0); 32];
        let mut fpr = [PhysReg(0); 32];
        for i in 0..32 {
            gpr[i] = PhysReg(i as u16);
            fpr[i] = PhysReg((32 + i) as u16);
        }
        Self { gpr, fpr }
    }

    /// Get the current physical register for an architectural register.
    /// x0 always returns PhysReg(0).
    #[inline]
    pub const fn get(&self, reg: RegIdx, is_fp: bool) -> PhysReg {
        let idx = reg.as_usize();
        if !is_fp && reg.is_zero() {
            return PhysReg(0);
        }
        if is_fp { self.fpr[idx] } else { self.gpr[idx] }
    }

    /// Update the mapping for an architectural register.
    /// No-op for x0 (always PhysReg(0)).
    #[inline]
    pub const fn set(&mut self, reg: RegIdx, is_fp: bool, p: PhysReg) {
        if !is_fp && reg.is_zero() {
            return; // x0 hardwired
        }
        let idx = reg.as_usize();
        if is_fp {
            self.fpr[idx] = p;
        } else {
            self.gpr[idx] = p;
        }
    }
}

impl Default for RenameMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_mapping() {
        let rm = RenameMap::new();
        for i in 0u8..32 {
            assert_eq!(rm.get(RegIdx::new(i), false), PhysReg(i as u16));
            assert_eq!(rm.get(RegIdx::new(i), true), PhysReg((32 + i) as u16));
        }
    }

    #[test]
    fn test_x0_always_phys_zero() {
        let mut rm = RenameMap::new();
        // x0 set is a no-op
        rm.set(RegIdx::new(0), false, PhysReg(99));
        assert_eq!(rm.get(RegIdx::new(0), false), PhysReg(0));
    }

    #[test]
    fn test_set_get_roundtrip() {
        let mut rm = RenameMap::new();
        rm.set(RegIdx::new(5), false, PhysReg(100));
        assert_eq!(rm.get(RegIdx::new(5), false), PhysReg(100));
        // FPR unaffected
        assert_eq!(rm.get(RegIdx::new(5), true), PhysReg(37));
    }

    #[test]
    fn test_fp_set_get() {
        let mut rm = RenameMap::new();
        rm.set(RegIdx::new(3), true, PhysReg(200));
        assert_eq!(rm.get(RegIdx::new(3), true), PhysReg(200));
        assert_eq!(rm.get(RegIdx::new(3), false), PhysReg(3)); // GPR unaffected
    }
}
