//! The results of translating an address, and the records a page walk or
//! `sfence.vma` carries to the point where it takes effect.

use crate::common::PhysAddr;
use crate::isa::privileged::Trap;
use crate::isa::reg::RegIdx;

/// Bits a hardware A/D update sets in a leaf PTE.
///
/// The update is conditional (privileged spec, Svadu): it applies only while
/// the PTE still holds the value the walk checked, apart from its A/D bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PteUpdate {
    /// Physical address of the PTE in memory.
    pub pte_addr: crate::common::PhysAddr,
    /// The PTE the walk checked.
    pub walked_pte: u64,
    /// The A/D bits to set.
    pub set_bits: u64,
}

/// The D-bit updates a store applies when it retires: one for each page it
/// writes (two when it crosses a page boundary).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyUpdates([Option<PteUpdate>; 2]);

impl DirtyUpdates {
    /// No update: the access sets no D bit.
    pub const NONE: Self = Self([None, None]);

    /// The updates for the access's first page and, when it crosses into
    /// one, its second.
    #[must_use]
    pub const fn of(first: Option<PteUpdate>, second: Option<PteUpdate>) -> Self {
        Self([first, second])
    }

    /// True when the access sets no D bit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Every update, first page first.
    pub fn iter(&self) -> impl Iterator<Item = &PteUpdate> {
        self.0.iter().flatten()
    }
}

/// The accessed (A) and dirty (D) bits of a PTE.
const PTE_AD_BITS: u64 = 0b1100_0000;

impl PteUpdate {
    /// The PTE to write over `current`, or `None` when the entry has changed
    /// since the walk in more than its A/D bits and the access must
    /// translate again.
    #[must_use]
    pub const fn applied_to(&self, current: u64) -> Option<u64> {
        if (current ^ self.walked_pte) & !PTE_AD_BITS != 0 {
            return None;
        }
        Some(current | self.set_bits)
    }
}

/// Deferred SFENCE.VMA operands for commit-time TLB invalidation.
///
/// SFENCE.VMA must not take effect speculatively — preceding PTE-modifying
/// stores may still be in the store buffer at execute time.  The operand
/// values are captured at execute and carried through the pipeline so the
/// commit stage can perform the correct selective (or global) TLB flush
/// after the store buffer has fully drained.
#[derive(Clone, Copy, Debug, Default)]
pub struct SfenceVmaInfo {
    /// Architectural index of rs1 (0 = flush all virtual addresses).
    pub rs1_idx: RegIdx,
    /// Architectural index of rs2 (0 = flush all ASIDs).
    pub rs2_idx: RegIdx,
    /// Value of rs1 (virtual address, when `rs1_idx` != 0).
    pub rs1_val: u64,
    /// Value of rs2 (ASID, when `rs2_idx` != 0).
    pub rs2_val: u64,
}

/// Result of a virtual-to-physical address translation operation.
///
/// This structure encapsulates the outcome of an MMU walk, including performance
/// metrics and any faults that may have occurred.
#[derive(Clone, Debug)]
pub struct TranslationResult {
    /// The translated physical address, or zero if translation failed.
    pub paddr: PhysAddr,
    /// Number of cycles consumed by the translation operation.
    pub cycles: u64,
    /// Trap that occurred during translation, if any.
    pub trap: Option<Trap>,
    /// The D-bit update a store applies when it retires (it must be exact).
    pub dirty_update: Option<PteUpdate>,
    /// The A-bit update the walker applies at once (it may be speculative).
    pub accessed_update: Option<PteUpdate>,
}

impl TranslationResult {
    /// Creates a successful translation result.
    #[inline]
    pub const fn success(paddr: PhysAddr, cycles: u64) -> Self {
        Self { paddr, cycles, trap: None, dirty_update: None, accessed_update: None }
    }

    /// Creates a translation result indicating a fault occurred.
    #[inline]
    pub const fn fault(trap: Trap, cycles: u64) -> Self {
        Self {
            paddr: PhysAddr(0),
            cycles,
            trap: Some(trap),
            dirty_update: None,
            accessed_update: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: u64 = 1 << 7;
    const A: u64 = 1 << 6;
    const LEAF: u64 = (0x8_0400 << 10) | 0b0100_0111;

    fn set_dirty() -> PteUpdate {
        PteUpdate { pte_addr: PhysAddr(0x8010_2000), walked_pte: LEAF, set_bits: A | D }
    }

    #[test]
    fn a_pte_update_sets_its_bits_on_the_walked_pte() {
        assert_eq!(set_dirty().applied_to(LEAF), Some(LEAF | D));
    }

    #[test]
    fn a_pte_update_ignores_a_and_d_changes_since_the_walk() {
        assert_eq!(set_dirty().applied_to(LEAF & !A), Some(LEAF | D));
    }

    #[test]
    fn a_pte_update_is_refused_once_the_mapping_changed() {
        let remapped = LEAF + (1 << 10);
        let revoked = LEAF & !0b100;

        assert_eq!(
            (set_dirty().applied_to(remapped), set_dirty().applied_to(revoked)),
            (None, None)
        );
    }

    #[test]
    fn test_translation_result() {
        let success = TranslationResult::success(PhysAddr(0x1000), 5);
        assert_eq!(success.paddr, PhysAddr(0x1000));
        assert_eq!(success.cycles, 5);
        assert!(success.trap.is_none());

        let fault = TranslationResult::fault(Trap::LoadPageFault(0x2000), 3);
        assert_eq!(fault.paddr, PhysAddr(0));
        assert_eq!(fault.cycles, 3);
        assert_eq!(fault.trap, Some(Trap::LoadPageFault(0x2000)));
    }
}
