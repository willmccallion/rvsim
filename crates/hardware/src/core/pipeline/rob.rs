//! Reorder Buffer (ROB) for out-of-order commit.
//!
//! The ROB is a circular buffer that tracks in-flight instructions from rename
//! through commit. It provides:
//! 1. **Allocation:** Assigns unique tags to instructions entering the backend.
//! 2. **Completion:** Marks instructions as done when their results are available.
//! 3. **In-order Commit:** Retires instructions from the head in program order.
//! 4. **Forwarding:** Provides the most recent result for any register from in-flight instructions.
//! 5. **Flush:** Squashes speculative entries after a misprediction or trap.

use crate::common::error::{ExceptionStage, Trap};
use crate::core::pipeline::signals::ControlSignals;

/// Unique tag identifying an in-flight instruction in the ROB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct RobTag(pub u32);

/// Lifecycle state of an ROB entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RobState {
    /// Entry allocated but instruction not yet finished executing.
    #[default]
    Issued,
    /// Execution complete, result available, waiting to commit.
    Completed,
    /// Instruction faulted; trap will be taken when it reaches ROB head.
    Faulted,
}

/// Deferred CSR write, applied only at commit time.
#[derive(Clone, Debug, Default)]
pub struct CsrUpdate {
    /// CSR address.
    pub addr: u32,
    /// Value of the CSR before the instruction.
    pub old_val: u64,
    /// New value to write at commit.
    pub new_val: u64,
}

/// A single entry in the Reorder Buffer.
#[derive(Clone, Debug, Default)]
pub struct RobEntry {
    /// Unique tag for this entry.
    pub tag: RobTag,
    /// Program counter of the instruction.
    pub pc: u64,
    /// Raw 32-bit instruction encoding.
    pub inst: u32,
    /// Instruction size in bytes (2 or 4).
    pub inst_size: u64,
    /// Destination register index.
    pub rd: usize,
    /// Whether rd is a floating-point register.
    pub rd_fp: bool,
    /// Computed result value (ALU output, load data, or link address).
    pub result: u64,
    /// Data for store instructions (rs2 value).
    pub store_data: u64,
    /// Virtual address for loads/stores (ALU output for memory ops).
    pub store_addr: u64,
    /// Control signals from decode.
    pub ctrl: ControlSignals,
    /// Current lifecycle state.
    pub state: RobState,
    /// Trap associated with this instruction, if faulted.
    pub trap: Option<Trap>,
    /// Pipeline stage where the exception was first detected.
    pub exception_stage: Option<ExceptionStage>,
    /// Deferred CSR write, if this is a CSR instruction.
    pub csr_update: Option<CsrUpdate>,
    /// Whether this entry is valid (occupied).
    pub valid: bool,
}

/// Reorder Buffer — circular buffer for in-order commit.
pub struct Rob {
    /// Fixed-size entry array.
    entries: Vec<RobEntry>,
    /// Index of the oldest entry (commit point).
    head: usize,
    /// Index where the next entry will be allocated.
    tail: usize,
    /// Number of valid entries.
    count: usize,
    /// Monotonically increasing tag counter.
    next_tag: u32,
}

impl Rob {
    /// Creates a new ROB with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let mut entries = Vec::with_capacity(capacity);
        entries.resize_with(capacity, RobEntry::default);
        Self {
            entries,
            head: 0,
            tail: 0,
            count: 0,
            next_tag: 1,
        }
    }

    /// Returns the ROB capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Returns the number of occupied entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns true if the ROB is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns true if the ROB is full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.count == self.entries.len()
    }

    /// Returns the number of free slots.
    #[inline]
    pub fn free_slots(&self) -> usize {
        self.entries.len() - self.count
    }

    /// Allocates a new ROB entry. Returns `None` if the ROB is full.
    pub fn allocate(
        &mut self,
        pc: u64,
        inst: u32,
        inst_size: u64,
        rd: usize,
        rd_fp: bool,
        ctrl: ControlSignals,
    ) -> Option<RobTag> {
        if self.is_full() {
            return None;
        }

        let tag = RobTag(self.next_tag);
        self.next_tag = self.next_tag.wrapping_add(1);
        if self.next_tag == 0 {
            self.next_tag = 1; // skip 0
        }

        self.entries[self.tail] = RobEntry {
            tag,
            pc,
            inst,
            inst_size,
            rd,
            rd_fp,
            result: 0,
            store_data: 0,
            store_addr: 0,
            ctrl,
            state: RobState::Issued,
            trap: None,
            exception_stage: None,
            csr_update: None,
            valid: true,
        };

        self.tail = (self.tail + 1) % self.entries.len();
        self.count += 1;
        Some(tag)
    }

    /// Marks an entry as Completed with its result value.
    pub fn complete(&mut self, tag: RobTag, result: u64) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.state = RobState::Completed;
            entry.result = result;
        }
    }

    /// Marks an entry as Faulted with a trap.
    pub fn fault(&mut self, tag: RobTag, trap: Trap, stage: ExceptionStage) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.state = RobState::Faulted;
            entry.trap = Some(trap);
            entry.exception_stage = Some(stage);
        }
    }

    /// Sets the CSR update for a given entry.
    pub fn set_csr_update(&mut self, tag: RobTag, update: CsrUpdate) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.csr_update = Some(update);
        }
    }

    /// Sets the store address and data for a given entry.
    pub fn set_store_info(&mut self, tag: RobTag, addr: u64, data: u64) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.store_addr = addr;
            entry.store_data = data;
        }
    }

    /// Returns a reference to the head entry (oldest), if the ROB is non-empty.
    pub fn peek_head(&self) -> Option<&RobEntry> {
        if self.count == 0 {
            None
        } else {
            Some(&self.entries[self.head])
        }
    }

    /// Returns a mutable reference to the head entry.
    pub fn peek_head_mut(&mut self) -> Option<&mut RobEntry> {
        if self.count == 0 {
            None
        } else {
            Some(&mut self.entries[self.head])
        }
    }

    /// Commits (retires) the head entry. Returns the entry if it was Completed or Faulted.
    /// Returns `None` if the ROB is empty or the head is still Issued.
    pub fn commit_head(&mut self) -> Option<RobEntry> {
        if self.count == 0 {
            return None;
        }

        let entry = &self.entries[self.head];
        if entry.state == RobState::Issued {
            return None; // not ready
        }

        let committed = self.entries[self.head].clone();
        self.entries[self.head].valid = false;
        self.head = (self.head + 1) % self.entries.len();
        self.count -= 1;
        Some(committed)
    }

    /// Flushes all entries from the ROB.
    pub fn flush_all(&mut self) {
        for entry in &mut self.entries {
            entry.valid = false;
        }
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }

    /// Flushes all entries allocated *after* the given tag (exclusive).
    /// The entry with `tag` itself is kept.
    pub fn flush_after(&mut self, tag: RobTag) {
        if self.count == 0 {
            return;
        }

        // Find the index of the entry with this tag
        let mut idx = self.head;
        let mut found = false;
        for _ in 0..self.count {
            if self.entries[idx].tag == tag {
                found = true;
                break;
            }
            idx = (idx + 1) % self.entries.len();
        }

        if !found {
            return;
        }

        // Keep entries from head through idx (inclusive), remove the rest
        let keep_idx = (idx + 1) % self.entries.len();
        let mut remove_idx = keep_idx;
        while remove_idx != self.tail {
            self.entries[remove_idx].valid = false;
            remove_idx = (remove_idx + 1) % self.entries.len();
        }

        self.tail = keep_idx;
        // Recount
        self.count = 0;
        let mut i = self.head;
        loop {
            if i == self.tail {
                break;
            }
            if self.entries[i].valid {
                self.count += 1;
            }
            i = (i + 1) % self.entries.len();
        }
    }

    /// Finds the latest in-flight result for a given register.
    /// Searches from tail backwards (most recent first).
    /// Returns `Some(value)` if a Completed entry writes to the register.
    pub fn find_latest_result(&self, reg: usize, is_fp: bool) -> Option<u64> {
        if self.count == 0 || (!is_fp && reg == 0) {
            return None;
        }

        // Walk backwards from tail-1 to head
        let mut idx = if self.tail == 0 {
            self.entries.len() - 1
        } else {
            self.tail - 1
        };

        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.rd == reg && entry.rd_fp == is_fp {
                if entry.state == RobState::Completed {
                    return Some(entry.result);
                }
                // Found the register but it's not ready yet — return None
                // to indicate a dependency (would stall or need bypass).
                return None;
            }
            if idx == 0 {
                idx = self.entries.len() - 1;
            } else {
                idx -= 1;
            }
        }

        None
    }

    /// Finds the latest in-flight value for a register, including Issued entries.
    /// This is used by rename to check if there's any pending write.
    /// Returns `Some((value, is_ready))` where is_ready indicates if the value is available.
    pub fn find_latest_producer(&self, reg: usize, is_fp: bool) -> Option<(u64, bool)> {
        if self.count == 0 || (!is_fp && reg == 0) {
            return None;
        }

        let mut idx = if self.tail == 0 {
            self.entries.len() - 1
        } else {
            self.tail - 1
        };

        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.rd == reg && entry.rd_fp == is_fp {
                let writes = if is_fp {
                    entry.ctrl.fp_reg_write
                } else {
                    entry.ctrl.reg_write
                };
                if writes {
                    let ready = entry.state == RobState::Completed;
                    return Some((entry.result, ready));
                }
            }
            if idx == 0 {
                idx = self.entries.len() - 1;
            } else {
                idx -= 1;
            }
        }

        None
    }

    /// Finds a mutable reference to the entry with the given tag.
    fn find_entry_mut(&mut self, tag: RobTag) -> Option<&mut RobEntry> {
        if self.count == 0 {
            return None;
        }

        let mut idx = self.head;
        for _ in 0..self.count {
            if self.entries[idx].valid && self.entries[idx].tag == tag {
                return Some(&mut self.entries[idx]);
            }
            idx = (idx + 1) % self.entries.len();
        }
        None
    }

    /// Iterate over all valid entries from head to tail, calling `f` on each.
    pub fn for_each_valid(&self, mut f: impl FnMut(&RobEntry)) {
        if self.count == 0 {
            return;
        }
        let mut idx = self.head;
        for _ in 0..self.count {
            if self.entries[idx].valid {
                f(&self.entries[idx]);
            }
            idx = (idx + 1) % self.entries.len();
        }
    }

    /// Finds a reference to the entry with the given tag.
    pub fn find_entry(&self, tag: RobTag) -> Option<&RobEntry> {
        if self.count == 0 {
            return None;
        }

        let mut idx = self.head;
        for _ in 0..self.count {
            if self.entries[idx].valid && self.entries[idx].tag == tag {
                return Some(&self.entries[idx]);
            }
            idx = (idx + 1) % self.entries.len();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::pipeline::signals::ControlSignals;

    fn make_ctrl(reg_write: bool, fp_reg_write: bool) -> ControlSignals {
        ControlSignals {
            reg_write,
            fp_reg_write,
            ..Default::default()
        }
    }

    #[test]
    fn test_allocate_and_commit() {
        let mut rob = Rob::new(4);
        assert!(rob.is_empty());
        assert_eq!(rob.free_slots(), 4);

        let tag = rob
            .allocate(0x1000, 0x13, 4, 1, false, make_ctrl(true, false))
            .unwrap();
        assert_eq!(rob.len(), 1);
        assert_eq!(rob.free_slots(), 3);

        // Can't commit while still Issued
        assert!(rob.commit_head().is_none());

        rob.complete(tag, 42);
        let entry = rob.commit_head().unwrap();
        assert_eq!(entry.pc, 0x1000);
        assert_eq!(entry.result, 42);
        assert_eq!(entry.state, RobState::Completed);
        assert!(rob.is_empty());
    }

    #[test]
    fn test_full_rob() {
        let mut rob = Rob::new(2);
        let _t1 = rob
            .allocate(0x1000, 0, 4, 1, false, make_ctrl(true, false))
            .unwrap();
        let _t2 = rob
            .allocate(0x1004, 0, 4, 2, false, make_ctrl(true, false))
            .unwrap();
        assert!(rob.is_full());
        assert!(
            rob.allocate(0x1008, 0, 4, 3, false, make_ctrl(true, false))
                .is_none()
        );
    }

    #[test]
    fn test_in_order_commit() {
        let mut rob = Rob::new(4);
        let t1 = rob
            .allocate(0x1000, 0, 4, 1, false, make_ctrl(true, false))
            .unwrap();
        let t2 = rob
            .allocate(0x1004, 0, 4, 2, false, make_ctrl(true, false))
            .unwrap();

        // Complete t2 first (out of order)
        rob.complete(t2, 200);
        // t1 is still Issued, so commit should fail
        assert!(rob.commit_head().is_none());

        // Now complete t1
        rob.complete(t1, 100);
        let e1 = rob.commit_head().unwrap();
        assert_eq!(e1.result, 100);

        let e2 = rob.commit_head().unwrap();
        assert_eq!(e2.result, 200);
    }

    #[test]
    fn test_fault_commit() {
        let mut rob = Rob::new(4);
        let t1 = rob
            .allocate(0x1000, 0, 4, 1, false, make_ctrl(true, false))
            .unwrap();
        rob.fault(t1, Trap::IllegalInstruction(0), ExceptionStage::Decode);

        let entry = rob.commit_head().unwrap();
        assert_eq!(entry.state, RobState::Faulted);
        assert!(entry.trap.is_some());
    }

    #[test]
    fn test_flush_all() {
        let mut rob = Rob::new(4);
        rob.allocate(0x1000, 0, 4, 1, false, make_ctrl(true, false));
        rob.allocate(0x1004, 0, 4, 2, false, make_ctrl(true, false));
        assert_eq!(rob.len(), 2);

        rob.flush_all();
        assert!(rob.is_empty());
        assert_eq!(rob.free_slots(), 4);
    }

    #[test]
    fn test_flush_after() {
        let mut rob = Rob::new(8);
        let t1 = rob
            .allocate(0x1000, 0, 4, 1, false, make_ctrl(true, false))
            .unwrap();
        let _t2 = rob
            .allocate(0x1004, 0, 4, 2, false, make_ctrl(true, false))
            .unwrap();
        let _t3 = rob
            .allocate(0x1008, 0, 4, 3, false, make_ctrl(true, false))
            .unwrap();
        assert_eq!(rob.len(), 3);

        rob.flush_after(t1);
        assert_eq!(rob.len(), 1);

        rob.complete(t1, 100);
        let entry = rob.commit_head().unwrap();
        assert_eq!(entry.pc, 0x1000);
    }

    #[test]
    fn test_find_latest_result() {
        let mut rob = Rob::new(8);
        let t1 = rob
            .allocate(0x1000, 0, 4, 5, false, make_ctrl(true, false))
            .unwrap();
        let t2 = rob
            .allocate(0x1004, 0, 4, 5, false, make_ctrl(true, false))
            .unwrap();

        rob.complete(t1, 100);
        rob.complete(t2, 200);

        // Should find t2's result (most recent)
        assert_eq!(rob.find_latest_result(5, false), Some(200));
        // x0 always returns None
        assert_eq!(rob.find_latest_result(0, false), None);
        // Non-existent register
        assert_eq!(rob.find_latest_result(10, false), None);
    }

    #[test]
    fn test_find_latest_result_not_ready() {
        let mut rob = Rob::new(8);
        rob.allocate(0x1000, 0, 4, 5, false, make_ctrl(true, false));
        // Entry is still Issued, so result is not ready
        assert_eq!(rob.find_latest_result(5, false), None);
    }

    #[test]
    fn test_csr_update() {
        let mut rob = Rob::new(4);
        let tag = rob
            .allocate(0x1000, 0, 4, 1, false, make_ctrl(true, false))
            .unwrap();
        rob.set_csr_update(
            tag,
            CsrUpdate {
                addr: 0x300,
                old_val: 10,
                new_val: 20,
            },
        );
        rob.complete(tag, 10);

        let entry = rob.commit_head().unwrap();
        let csr = entry.csr_update.unwrap();
        assert_eq!(csr.addr, 0x300);
        assert_eq!(csr.new_val, 20);
    }

    #[test]
    fn test_circular_wraparound() {
        let mut rob = Rob::new(2);

        // Fill and drain several times to test wraparound
        for i in 0..10 {
            let tag = rob
                .allocate(i * 4, 0, 4, 1, false, make_ctrl(true, false))
                .unwrap();
            rob.complete(tag, i);
            let entry = rob.commit_head().unwrap();
            assert_eq!(entry.result, i);
        }
    }
}
