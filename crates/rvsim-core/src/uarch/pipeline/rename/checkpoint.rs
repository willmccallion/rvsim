//! Checkpoint table for O(1) branch recovery.
//!
//! A `CheckpointTable` stores snapshots of the speculative rename map taken
//! at branch/jump dispatch time. On a misprediction, the rename map is
//! restored directly from the checkpoint instead of walking the entire
//! surviving ROB (`rebuild_rename_map()`), reducing recovery from O(ROB size)
//! to O(1).

use super::map::RenameMap;
use crate::uarch::pipeline::rob::RobTag;

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
}

/// A free slot a [`CheckpointTable::reserve`] call set aside.
#[derive(Debug)]
pub struct ReservedCheckpoint<'a> {
    table: &'a mut CheckpointTable,
    index: usize,
}

impl ReservedCheckpoint<'_> {
    /// Saves `rename_map` for `branch_tag` in the reserved slot.
    pub fn fill(self, branch_tag: RobTag, rename_map: &RenameMap) -> CheckpointId {
        self.table.slots[self.index] =
            Some(Checkpoint { branch_tag, rename_map: rename_map.clone() });
        self.table.count += 1;
        CheckpointId(self.index as u8)
    }
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

    #[cfg(test)]
    /// Returns true if all slots are occupied.
    #[inline]
    pub const fn is_full(&self) -> bool {
        self.count == self.slots.len()
    }

    #[cfg(test)]
    /// Returns the number of free slots.
    #[inline]
    pub const fn available(&self) -> usize {
        self.slots.len() - self.count
    }

    /// Reserves the first free slot, or `None` if the table is full. The
    /// reservation borrows the table, so the slot is still free when
    /// [`ReservedCheckpoint::fill`] writes it.
    pub fn reserve(&mut self) -> Option<ReservedCheckpoint<'_>> {
        let index = self.slots.iter().position(Option::is_none)?;
        Some(ReservedCheckpoint { table: self, index })
    }

    #[cfg(test)]
    /// Allocates a checkpoint slot saving `rename_map` for `branch_tag`.
    /// Returns `None` if the table is full.
    pub fn allocate(&mut self, branch_tag: RobTag, rename_map: &RenameMap) -> Option<CheckpointId> {
        Some(self.reserve()?.fill(branch_tag, rename_map))
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
        self.slots.fill(None);
        self.count = 0;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::isa::reg::RegIdx;
    use crate::uarch::pipeline::rename::prf::PhysReg;

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
        let tag = RobTag::new(10);
        let id = table.allocate(tag, &rm).unwrap();
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
        table.allocate(RobTag::new(1), &rm).unwrap();
        table.allocate(RobTag::new(2), &rm).unwrap();
        assert!(table.is_full());
        assert!(table.allocate(RobTag::new(3), &rm).is_none());
    }

    #[test]
    fn test_flush_after() {
        let mut table = CheckpointTable::new(4);
        let rm = make_rename_map(1);
        table.allocate(RobTag::new(1), &rm).unwrap();
        table.allocate(RobTag::new(2), &rm).unwrap();
        table.allocate(RobTag::new(3), &rm).unwrap();
        table.allocate(RobTag::new(4), &rm).unwrap();
        assert!(table.is_full());

        // Keep tag 2, flush tags 3 and 4
        table.flush_after(RobTag::new(2));
        assert_eq!(table.available(), 2);
        assert!(table.find_by_tag(RobTag::new(1)).is_some());
        assert!(table.find_by_tag(RobTag::new(2)).is_some());
        assert!(table.find_by_tag(RobTag::new(3)).is_none());
        assert!(table.find_by_tag(RobTag::new(4)).is_none());
    }

    #[test]
    fn test_flush_all() {
        let mut table = CheckpointTable::new(4);
        let rm = make_rename_map(1);
        table.allocate(RobTag::new(1), &rm).unwrap();
        table.allocate(RobTag::new(2), &rm).unwrap();
        table.flush_all();
        assert_eq!(table.available(), 4);
        assert!(table.find_by_tag(RobTag::new(1)).is_none());
    }

    #[test]
    fn test_zero_capacity() {
        let mut table = CheckpointTable::new(0);
        assert!(table.is_full());
        assert_eq!(table.available(), 0);
        let rm = make_rename_map(1);
        assert!(table.allocate(RobTag::new(1), &rm).is_none());
        // flush_after and flush_all should be no-ops
        table.flush_after(RobTag::new(1));
        table.flush_all();
    }
}
