//! LR/SC reservation records.

/// Deferred LR/SC reservation action for commit-time application.
///
/// LR/SC must not modify the load reservation speculatively — if the
/// instruction is squashed, the reservation state would be corrupted.
/// Instead, Memory2 records the intended action here, and the commit
/// stage applies it when the instruction retires.
#[derive(Clone, Copy, Debug)]
pub enum LrScRecord {
    /// LR: set the reservation to this physical address at commit.
    Lr {
        /// Physical address to reserve.
        paddr: crate::common::PhysAddr,
    },
}
