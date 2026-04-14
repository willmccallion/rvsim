//! Checkpoint table for O(1) branch recovery.
//!
//! A `CheckpointTable` stores snapshots of the speculative rename map taken
//! at branch/jump dispatch time. On a misprediction, the rename map is
//! restored directly from the checkpoint instead of walking the entire
//! surviving ROB (`rebuild_rename_map()`), reducing recovery from O(ROB size)
//! to O(1).

use super::rename_map::RenameMap;
use super::rob::RobTag;

/// Index into the checkpoint table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointId(pub u8);

/// A single rename map snapshot associated with a branch/jump.
#[derive(Clone, Debug)]
pub struct Checkpoint {
    /// ROB tag of the branch/jump that owns this checkpoint.
    pub branch_tag: RobTag,
    /// Rename map snapshot taken *after* the branch's own rd rename.
    pub rename_map: RenameMap,
    /// `vtype` CSR at branch dispatch time (for vsetvl rollback on misprediction).
    pub vtype: u64,
    /// `vl` CSR at branch dispatch time.
    pub vl: u64,
    /// `frm` CSR (FP rounding mode) at branch dispatch time.
    pub frm: u64,
    /// `vxrm` CSR (vector fixed-point rounding mode) at branch dispatch time.
    pub vxrm: u64,
}

/// Fixed-size table of checkpoint slots.
#[derive(Debug)]
pub struct CheckpointTable {
    slots: Vec<Option<Checkpoint>>,
    count: usize,
}

impl CheckpointTable {
    /// Creates a new checkpoint table with `capacity` slots.
    pub fn new(capacity: usize) -> Self {
        let mut slots = Vec::with_capacity(capacity);
        slots.resize_with(capacity, || None);
        Self { slots, count: 0 }
    }

    /// Returns the table capacity.
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Returns true if all slots are occupied.
    #[inline]
    pub const fn is_full(&self) -> bool {
        self.count == self.slots.len()
    }

    /// Returns the number of free slots.
    #[inline]
    pub const fn available(&self) -> usize {
        self.slots.len() - self.count
    }

    /// Allocates a checkpoint slot, saving `rename_map`, `vtype`, `vl`, `frm`, and `vxrm`
    /// for `branch_tag`. Returns `None` if the table is full.
    pub fn allocate(
        &mut self,
        branch_tag: RobTag,
        rename_map: &RenameMap,
        vtype: u64,
        vl: u64,
        frm: u64,
        vxrm: u64,
    ) -> Option<CheckpointId> {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(Checkpoint {
                    branch_tag,
                    rename_map: rename_map.clone(),
                    vtype,
                    vl,
                    frm,
                    vxrm,
                });
                self.count += 1;
                return Some(CheckpointId(i as u8));
            }
        }
        None
    }

    /// Finds the checkpoint with the given `branch_tag`.
    pub fn find_by_tag(&self, tag: RobTag) -> Option<&Checkpoint> {
        self.slots.iter().filter_map(|s| s.as_ref()).find(|c| c.branch_tag == tag)
    }

    /// Frees the checkpoint at `id`.
    pub fn free(&mut self, id: CheckpointId) {
        let idx = id.0 as usize;
        if idx < self.slots.len() && self.slots[idx].is_some() {
            self.slots[idx] = None;
            self.count -= 1;
        }
    }

    /// Frees all checkpoints whose `branch_tag` is newer than `keep_tag`.
    pub fn flush_after(&mut self, keep_tag: RobTag) {
        for slot in &mut self.slots {
            if let &mut Some(ref ckpt) = slot
                && ckpt.branch_tag.is_newer_than(keep_tag)
            {
                *slot = None;
                self.count -= 1;
            }
        }
    }

    /// Frees all checkpoint slots.
    pub fn flush_all(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
        self.count = 0;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::RegIdx;
    use crate::core::pipeline::prf::PhysReg;

    fn make_rename_map(marker: u16) -> RenameMap {
        let mut rm = RenameMap::new();
        rm.set(RegIdx::new(1), false, PhysReg(marker));
        rm
    }

    #[test]
    fn test_allocate_and_find() {
        let mut table = CheckpointTable::new(4);
        assert_eq!(table.available(), 4);
        assert!(!table.is_full());

        let rm = make_rename_map(100);
        let tag = RobTag(10);
        let id = table.allocate(tag, &rm, 0, 0, 0, 0).unwrap();
        assert_eq!(table.available(), 3);

        let ckpt = table.find_by_tag(tag).unwrap();
        assert_eq!(ckpt.branch_tag, tag);
        assert_eq!(ckpt.rename_map.get(RegIdx::new(1), false), PhysReg(100));

        table.free(id);
        assert_eq!(table.available(), 4);
        assert!(table.find_by_tag(tag).is_none());
    }

    #[test]
    fn test_full_table() {
        let mut table = CheckpointTable::new(2);
        let rm = make_rename_map(1);
        table.allocate(RobTag(1), &rm, 0, 0, 0, 0).unwrap();
        table.allocate(RobTag(2), &rm, 0, 0, 0, 0).unwrap();
        assert!(table.is_full());
        assert!(table.allocate(RobTag(3), &rm, 0, 0, 0, 0).is_none());
    }

    #[test]
    fn test_flush_after() {
        let mut table = CheckpointTable::new(4);
        let rm = make_rename_map(1);
        table.allocate(RobTag(1), &rm, 0, 0, 0, 0).unwrap();
        table.allocate(RobTag(2), &rm, 0, 0, 0, 0).unwrap();
        table.allocate(RobTag(3), &rm, 0, 0, 0, 0).unwrap();
        table.allocate(RobTag(4), &rm, 0, 0, 0, 0).unwrap();
        assert!(table.is_full());

        // Keep tag 2, flush tags 3 and 4
        table.flush_after(RobTag(2));
        assert_eq!(table.available(), 2);
        assert!(table.find_by_tag(RobTag(1)).is_some());
        assert!(table.find_by_tag(RobTag(2)).is_some());
        assert!(table.find_by_tag(RobTag(3)).is_none());
        assert!(table.find_by_tag(RobTag(4)).is_none());
    }

    #[test]
    fn test_flush_all() {
        let mut table = CheckpointTable::new(4);
        let rm = make_rename_map(1);
        table.allocate(RobTag(1), &rm, 0, 0, 0, 0).unwrap();
        table.allocate(RobTag(2), &rm, 0, 0, 0, 0).unwrap();
        table.flush_all();
        assert_eq!(table.available(), 4);
        assert!(table.find_by_tag(RobTag(1)).is_none());
    }

    #[test]
    fn test_zero_capacity() {
        let mut table = CheckpointTable::new(0);
        assert!(table.is_full());
        assert_eq!(table.available(), 0);
        let rm = make_rename_map(1);
        assert!(table.allocate(RobTag(1), &rm, 0, 0, 0, 0).is_none());
        // flush_after and flush_all should be no-ops
        table.flush_after(RobTag(1));
        table.flush_all();
    }
}
