//! Speculative rename map: architectural register → current physical register.
//!
//! Tracks the current mapping from architectural register names to physical
//! register numbers. Maintained speculatively; the `committed_rename_map`
//! in `O3Engine` tracks the last committed state for flush recovery.

use crate::isa::reg::{ArchReg, RegIdx};
use crate::isa::rvv::MAX_GROUP_REGS;
use crate::isa::rvv::VRegIdx;
use crate::uarch::pipeline::rename::vec_prf::{VecPhysReg, VecRegGroup};

use super::prf::PhysReg;

/// Speculative rename map for GPRs, FPRs, and VPRs.
#[derive(Clone, Debug)]
pub struct RenameMap {
    /// GPR rename map: gpr[i] = current physical reg for `x_i`
    gpr: [PhysReg; RegIdx::COUNT],
    /// FPR rename map: fpr[i] = current physical reg for `f_i`
    fpr: [PhysReg; RegIdx::COUNT],
    /// VPR rename map: vpr[i] = current physical vec reg for `v_i`
    vpr: [VecPhysReg; VRegIdx::COUNT],
}

impl RenameMap {
    /// Create a new rename map with identity mapping.
    /// GPR `i` maps to physical register `i`, FPR `i` to `32 + i`, and
    /// vector register `i` to vector physical register `i`.
    pub fn new() -> Self {
        Self {
            gpr: std::array::from_fn(PhysReg::from_index),
            fpr: std::array::from_fn(|i| PhysReg::from_index(RegIdx::COUNT + i)),
            vpr: std::array::from_fn(VecPhysReg::from_index),
        }
    }

    /// Get the current physical register for an architectural register.
    /// x0 always returns [`PhysReg::ZERO`].
    #[inline]
    pub const fn get(&self, reg: ArchReg) -> PhysReg {
        match reg {
            ArchReg::Int(idx) if idx.is_zero() => PhysReg::ZERO,
            ArchReg::Int(idx) => self.gpr[idx.as_usize()],
            ArchReg::Fp(idx) => self.fpr[idx.as_usize()],
        }
    }

    /// Update the mapping for an architectural register.
    /// No-op for x0 (always [`PhysReg::ZERO`]).
    #[inline]
    pub const fn set(&mut self, reg: ArchReg, p: PhysReg) {
        match reg {
            ArchReg::Int(idx) if idx.is_zero() => {}
            ArchReg::Int(idx) => self.gpr[idx.as_usize()] = p,
            ArchReg::Fp(idx) => self.fpr[idx.as_usize()] = p,
        }
    }

    /// Get the current physical vector register for an architectural vector register.
    #[inline]
    pub const fn get_vec(&self, vreg: VRegIdx) -> VecPhysReg {
        self.vpr[vreg.as_usize()]
    }

    /// Update the mapping for an architectural vector register.
    /// No zero-register skip (vectors have no hardwired zero register).
    #[inline]
    pub const fn set_vec(&mut self, vreg: VRegIdx, p: VecPhysReg) {
        self.vpr[vreg.as_usize()] = p;
    }

    /// The physical registers the `count` vector registers from `base` map
    /// to.
    pub fn vec_group(&self, base: VRegIdx, count: u8) -> VecRegGroup {
        let mut regs = [VecPhysReg::ZERO; MAX_GROUP_REGS];
        for (offset, slot) in regs.iter_mut().enumerate().take(count as usize) {
            *slot = self.get_vec(base.offset(offset));
        }
        VecRegGroup::from_slice(&regs[..count as usize])
    }

    /// Maps the vector registers from `base` to `group`'s in order,
    /// returning the group they mapped to before.
    pub fn rename_vec_group(&mut self, base: VRegIdx, group: VecRegGroup) -> VecRegGroup {
        let old = self.vec_group(base, group.len());
        for (offset, &reg) in group.as_slice().iter().enumerate() {
            self.set_vec(base.offset(offset), reg);
        }
        old
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
    use crate::isa::reg::RegIdx;

    #[test]
    fn test_identity_mapping() {
        let rm = RenameMap::new();
        for i in 0u8..32 {
            assert_eq!(rm.get(ArchReg::Int(RegIdx::new(i))), PhysReg::from_index(usize::from(i)));
            assert_eq!(
                rm.get(ArchReg::Fp(RegIdx::new(i))),
                PhysReg::from_index(usize::from(32 + i))
            );
        }
    }

    #[test]
    fn test_x0_always_phys_zero() {
        let mut rm = RenameMap::new();
        // x0 set is a no-op
        rm.set(ArchReg::Int(RegIdx::new(0)), PhysReg::from_index(99));
        assert_eq!(rm.get(ArchReg::Int(RegIdx::new(0))), PhysReg::ZERO);
    }

    #[test]
    fn test_set_get_roundtrip() {
        let mut rm = RenameMap::new();
        rm.set(ArchReg::Int(RegIdx::new(5)), PhysReg::from_index(100));
        assert_eq!(rm.get(ArchReg::Int(RegIdx::new(5))), PhysReg::from_index(100));
        // FPR unaffected
        assert_eq!(rm.get(ArchReg::Fp(RegIdx::new(5))), PhysReg::from_index(37));
    }

    #[test]
    fn test_fp_set_get() {
        let mut rm = RenameMap::new();
        rm.set(ArchReg::Fp(RegIdx::new(3)), PhysReg::from_index(200));
        assert_eq!(rm.get(ArchReg::Fp(RegIdx::new(3))), PhysReg::from_index(200));
        assert_eq!(rm.get(ArchReg::Int(RegIdx::new(3))), PhysReg::from_index(3)); // GPR unaffected
    }

    #[test]
    fn test_vec_identity_mapping() {
        let rm = RenameMap::new();
        for i in 0u8..32 {
            assert_eq!(rm.get_vec(VRegIdx::new(i)), VecPhysReg::from_index(usize::from(i)));
        }
    }

    #[test]
    fn test_vec_set_get() {
        let mut rm = RenameMap::new();
        let v5 = VRegIdx::new(5);
        rm.set_vec(v5, VecPhysReg::from_index(40));
        assert_eq!(rm.get_vec(v5), VecPhysReg::from_index(40));
        // v0 can be remapped (no hardwired zero for vectors)
        let v0 = VRegIdx::new(0);
        rm.set_vec(v0, VecPhysReg::from_index(50));
        assert_eq!(rm.get_vec(v0), VecPhysReg::from_index(50));
    }
}
