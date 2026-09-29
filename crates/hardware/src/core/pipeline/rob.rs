//! Reorder Buffer (ROB) for out-of-order commit.
//!
//! The ROB is a circular buffer that tracks in-flight instructions from rename
//! through commit. It provides:
//! 1. **Allocation:** Assigns unique tags to instructions entering the backend.
//! 2. **Completion:** Marks instructions as done when their results are available.
//! 3. **In-order Commit:** Retires instructions from the head in program order.
//! 4. **Forwarding:** Provides the most recent result for any register from in-flight instructions.
//! 5. **Flush:** Squashes speculative entries after a misprediction or trap.

use crate::sim::state::write_log::WriteSeq;
use std::collections::HashMap;

use crate::common::error::{DirtyUpdates, ExceptionStage, LrScRecord, SfenceVmaInfo, Trap};
use crate::common::{CsrAddr, InstSeq, InstSize, RegIdx};
use crate::core::exec::execute::CsrWrite;
use crate::core::exec::signals::ControlSignals;
use crate::core::pipeline::checkpoint::CheckpointId;
use crate::core::pipeline::prf::PhysReg;
use crate::core::units::vpu::shadow::{ElementWrite, VectorWrites};
use crate::core::units::vpu::types::VecPhysReg;
use crate::core::units::vpu::types::VectorConfig;

/// Branch outcome recorded at execute time for deferred predictor update.
///
/// Grouping `taken` and `mispredicted` into a struct prevents accidentally
/// swapping the two adjacent bools at call sites.
#[derive(Clone, Copy, Debug, Default)]
pub struct BpOutcome {
    /// Whether the branch was actually taken.
    pub taken: bool,
    /// Whether the branch was mispredicted (i.e. fetch used the wrong path).
    pub mispredicted: bool,
}

/// Unique tag identifying an in-flight instruction in the ROB.
///
/// Tags are monotonically increasing (wrapping at `u32::MAX` back to 1,
/// skipping 0). Comparisons between in-flight tags must use
/// [`RobTag::is_older_than`] / [`RobTag::is_newer_than`] which handle
/// wraparound via signed-distance arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct RobTag(pub u32);

impl RobTag {
    /// Returns true if `self` is older (was allocated before) `other`.
    ///
    /// Uses wrapping subtraction so that the comparison remains correct
    /// across the `u32` wraparound boundary, as long as the distance
    /// between any two live tags is less than `2^31` (always true for
    /// realistic ROB sizes).
    #[inline]
    pub const fn is_older_than(self, other: Self) -> bool {
        (self.0.wrapping_sub(other.0) as i32) < 0
    }

    /// Returns true if `self` is newer (was allocated after) `other`.
    #[inline]
    pub const fn is_newer_than(self, other: Self) -> bool {
        other.is_older_than(self)
    }

    /// Returns true if `self` is older than or equal to `other`.
    #[inline]
    pub const fn is_older_or_eq(self, other: Self) -> bool {
        !other.is_older_than(self)
    }

    /// Orders in-flight tags oldest first.
    #[must_use]
    pub fn age_cmp(self, other: Self) -> std::cmp::Ordering {
        (self.0.wrapping_sub(other.0).cast_signed()).cmp(&0)
    }
}

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
    pub addr: CsrAddr,
    /// Value of the CSR before the instruction.
    pub old_val: u64,
    /// New value to write at commit.
    pub new_val: u64,
    /// Whether this CSR write has already been applied (e.g. at complete time for O3).
    pub applied: bool,
}

impl From<CsrWrite> for CsrUpdate {
    fn from(write: CsrWrite) -> Self {
        Self { addr: write.addr, old_val: write.old, new_val: write.new, applied: false }
    }
}

/// A single entry in the Reorder Buffer.
#[derive(Clone, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct RobEntry {
    /// Unique tag for this entry.
    pub tag: RobTag,
    /// The instruction's place in fetch order.
    pub seq: InstSeq,
    /// Program counter of the instruction.
    pub pc: u64,
    /// Raw 32-bit instruction encoding.
    pub inst: u32,
    /// Instruction size in bytes (2 or 4).
    pub inst_size: InstSize,
    /// Destination register index.
    pub rd: RegIdx,
    /// Whether rd is a floating-point register.
    pub rd_fp: bool,
    /// Computed result value (ALU output, load data, or link address).
    /// `None` while the instruction is still executing (`Issued` state);
    /// `Some(value)` once the instruction completes.
    pub result: Option<u64>,
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
    /// The vector configuration a `vsetvl` sets, written to the CSRs at commit.
    pub vec_csr_update: Option<VectorConfig>,
    /// The element a vector memory instruction faulted on: `vstart` when
    /// its trap is taken.
    pub fault_vstart: Option<u64>,
    /// The `vl` a fault-only-first load trimmed itself to, written at commit.
    pub vl_trim: Option<u64>,
    /// Whether this entry is valid (occupied).
    pub valid: bool,
    /// Physical register allocated for rd at rename (O3 backend).
    pub phys_dst: PhysReg,
    /// Previous mapping for rd — returned to free list at commit (O3 backend).
    pub old_phys_dst: PhysReg,
    /// FP exception flags generated by this instruction (deferred to commit).
    pub fp_flags: u8,
    /// A branch or jump that resolved; commit counts its prediction.
    pub control_resolved: bool,
    /// Branch outcome recorded at execute time (taken + mispredicted).
    pub bp_outcome: BpOutcome,
    /// Where a taken branch or a jump goes; `None` for not-taken.
    pub bp_target: Option<u64>,
    /// The D-bit updates the access applies when it retires.
    pub dirty_updates: DirtyUpdates,
    /// Deferred SFENCE.VMA operands for commit-time TLB invalidation.
    pub sfence_vma: Option<SfenceVmaInfo>,
    /// Deferred LR/SC reservation action for commit-time application.
    pub lr_sc: Option<LrScRecord>,
    /// Write-log position when a load, LR or AMO read its value from RAM;
    /// commit re-validates LR and AMO against it.
    pub observed: Option<WriteSeq>,
    /// Checkpoint table slot allocated for this branch/jump (O3 backend).
    pub checkpoint_id: Option<CheckpointId>,
    /// Physical vector registers allocated for destination LMUL group (O3 backend).
    pub vec_phys_dst: [VecPhysReg; 8],
    /// Previous physical mappings for destination LMUL group (reclaimed at commit).
    pub vec_old_phys_dst: [VecPhysReg; 8],
    /// Number of destination vector registers in the LMUL group (0 for non-vector).
    pub vec_dst_count: u8,
    /// Deferred vxsat (fixed-point saturation) flag from vector execution (applied at commit).
    pub vxsat: bool,
    /// Vector register writes a backend without vector renaming holds
    /// back for commit; `Some` marks the entry as an executed vector op.
    pub vec_writes: Option<Box<VectorWrites>>,
}

/// Reorder Buffer — circular buffer for in-order commit.
#[derive(Debug)]
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
    /// O(1) tag → slot index lookup.
    tag_index: HashMap<RobTag, usize>,
    /// Valid entries carrying a `vec_csr_update`.
    vec_config_updates: usize,
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
            tag_index: HashMap::with_capacity(capacity),
            vec_config_updates: 0,
        }
    }

    /// Returns the ROB capacity.
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Returns the number of occupied entries.
    #[inline]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Returns true if the ROB is empty.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns true if the ROB is full.
    #[inline]
    pub const fn is_full(&self) -> bool {
        self.count == self.entries.len()
    }

    /// Returns the number of free slots.
    #[inline]
    pub const fn free_slots(&self) -> usize {
        self.entries.len() - self.count
    }

    /// Allocates a new ROB entry. Returns `None` if the ROB is full.
    #[allow(clippy::too_many_arguments)]
    pub fn allocate(
        &mut self,
        pc: u64,
        inst: u32,
        inst_size: InstSize,
        rd: RegIdx,
        rd_fp: bool,
        ctrl: ControlSignals,
        phys_dst: PhysReg,
        old_phys_dst: PhysReg,
        seq: InstSeq,
    ) -> Option<RobTag> {
        if self.is_full() {
            return None;
        }

        let tag = RobTag(self.next_tag);
        self.next_tag = self.next_tag.wrapping_add(1);
        if self.next_tag == 0 {
            self.next_tag = 1;
        }

        self.entries[self.tail] = RobEntry {
            tag,
            seq,
            pc,
            inst,
            inst_size,
            rd,
            rd_fp,
            result: None,
            store_data: 0,
            store_addr: 0,
            ctrl,
            state: RobState::Issued,
            trap: None,
            exception_stage: None,
            csr_update: None,
            vec_csr_update: None,
            fault_vstart: None,
            vl_trim: None,
            valid: true,
            phys_dst,
            old_phys_dst,
            fp_flags: 0,
            control_resolved: false,
            bp_outcome: BpOutcome::default(),
            bp_target: None,
            dirty_updates: DirtyUpdates::NONE,
            sfence_vma: None,
            lr_sc: None,
            observed: None,
            checkpoint_id: None,
            vec_phys_dst: [VecPhysReg::ZERO; 8],
            vec_old_phys_dst: [VecPhysReg::ZERO; 8],
            vec_dst_count: 0,
            vxsat: false,
            vec_writes: None,
        };

        let _ = self.tag_index.insert(tag, self.tail);
        self.tail = (self.tail + 1) % self.entries.len();
        self.count += 1;
        Some(tag)
    }

    /// Marks an entry as Completed with its result value.
    ///
    /// Does nothing if the entry is already Faulted — a fault set during
    /// execute must not be overwritten by a later writeback completion.
    pub fn complete(&mut self, tag: RobTag, result: u64) {
        if let Some(entry) = self.find_entry_mut(tag)
            && entry.state != RobState::Faulted
        {
            entry.state = RobState::Completed;
            entry.result = Some(result);
        }
    }

    /// Records an entry's result the cycle its unit produces it, before the
    /// entry reaches writeback: the bypass a dependent reads from.
    pub fn forward(&mut self, tag: RobTag, result: u64) {
        if let Some(entry) = self.find_entry_mut(tag)
            && entry.state == RobState::Issued
        {
            entry.result = Some(result);
        }
    }

    /// Marks an entry as Faulted with a trap. The first fault an
    /// instruction raises is the one it takes: a later stage cannot replace
    /// it, since the instruction never really reached that stage.
    pub fn fault(&mut self, tag: RobTag, trap: Trap, stage: ExceptionStage) {
        if let Some(entry) = self.find_entry_mut(tag)
            && entry.state != RobState::Faulted
        {
            entry.state = RobState::Faulted;
            entry.trap = Some(trap);
            entry.exception_stage = Some(stage);
        }
    }

    /// True when `tag` is the oldest instruction in the ROB, so everything
    /// before it has committed.
    #[must_use]
    pub fn is_head(&self, tag: RobTag) -> bool {
        self.peek_head().is_some_and(|head| head.tag == tag)
    }

    /// Faults a vector memory instruction at `element`, where its trap
    /// sets `vstart`.
    pub fn fault_element(&mut self, tag: RobTag, trap: Trap, stage: ExceptionStage, element: u64) {
        if let Some(entry) = self.find_entry_mut(tag)
            && entry.state != RobState::Faulted
        {
            entry.state = RobState::Faulted;
            entry.trap = Some(trap);
            entry.exception_stage = Some(stage);
            entry.fault_vstart = Some(element);
        }
    }

    /// Records the `vl` a fault-only-first load trims itself to.
    pub fn set_vl_trim(&mut self, tag: RobTag, vl: u64) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.vl_trim = Some(entry.vl_trim.map_or(vl, |current| current.min(vl)));
        }
    }

    /// Records the vector configuration a `vsetvl` establishes.
    pub fn set_vec_csr_update(&mut self, tag: RobTag, config: VectorConfig) {
        if let Some(entry) = self.find_entry_mut(tag) {
            let first_update = entry.vec_csr_update.is_none();
            entry.vec_csr_update = Some(config);
            if first_update {
                self.vec_config_updates += 1;
            }
        }
    }

    /// The configuration the youngest executed, uncommitted `vsetvl` set:
    /// what instructions younger than it run under.
    #[must_use]
    pub fn youngest_vec_csr_update(&self) -> Option<VectorConfig> {
        if self.vec_config_updates == 0 {
            return None;
        }
        let len = self.entries.len();
        let mut idx = (self.tail + len - 1) % len;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.vec_csr_update.is_some() {
                return entry.vec_csr_update;
            }
            idx = (idx + len - 1) % len;
        }
        None
    }

    /// Sets the CSR update for a given entry.
    pub fn set_csr_update(&mut self, tag: RobTag, update: CsrUpdate) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.csr_update = Some(update);
        }
    }

    /// Marks the CSR update for a given entry as already applied (so commit skips it).
    pub fn mark_csr_applied(&mut self, tag: RobTag) {
        if let Some(entry) = self.find_entry_mut(tag)
            && let Some(ref mut csr_update) = entry.csr_update
        {
            csr_update.applied = true;
        }
    }

    /// Records a branch's or jump's resolved outcome for commit.
    pub fn set_control_outcome(&mut self, tag: RobTag, outcome: BpOutcome, target: Option<u64>) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.control_resolved = true;
            entry.bp_outcome = outcome;
            entry.bp_target = target;
        }
    }

    /// Sets the FP exception flags for a given entry (accumulated at commit).
    pub fn set_fp_flags(&mut self, tag: RobTag, fp_flags: u8) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.fp_flags |= fp_flags;
        }
    }

    /// Records the registers a vector instruction wrote, for commit to land.
    pub fn set_vec_writes(&mut self, tag: RobTag, writes: VectorWrites) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.vec_writes = Some(Box::new(writes));
        }
    }

    /// Records one element a vector load returned, for commit to land.
    pub fn push_vec_element_write(&mut self, tag: RobTag, write: ElementWrite) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.vec_writes.get_or_insert_with(Box::default).elements.push(write);
        }
    }

    /// Sets the deferred vxsat (fixed-point saturation) flag for a vector instruction.
    pub fn set_vxsat(&mut self, tag: RobTag, val: bool) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.vxsat |= val;
        }
    }

    /// Files the D-bit updates an entry applies when it retires.
    pub fn set_dirty_updates(&mut self, tag: RobTag, updates: DirtyUpdates) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.dirty_updates = updates;
        }
    }

    /// Attaches deferred SFENCE.VMA operands to a ROB entry.
    pub fn set_sfence_vma(&mut self, tag: RobTag, info: SfenceVmaInfo) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.sfence_vma = Some(info);
        }
    }

    /// Attaches a deferred LR/SC reservation action to a ROB entry.
    pub fn set_lr_sc(&mut self, tag: RobTag, record: LrScRecord) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.lr_sc = Some(record);
        }
    }

    /// Records the write-log position at which a memory read took its value.
    pub fn set_observed(&mut self, tag: RobTag, seq: WriteSeq) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.observed = Some(seq);
        }
    }

    /// Sets the checkpoint ID for a given entry (branch/jump at dispatch).
    pub fn set_checkpoint_id(&mut self, tag: RobTag, id: CheckpointId) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.checkpoint_id = Some(id);
        }
    }

    /// Sets the vector physical destination registers for a given entry.
    pub fn set_vec_phys_dst(
        &mut self,
        tag: RobTag,
        phys_dst: [VecPhysReg; 8],
        old_phys_dst: [VecPhysReg; 8],
        count: u8,
    ) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.vec_phys_dst = phys_dst;
            entry.vec_old_phys_dst = old_phys_dst;
            entry.vec_dst_count = count;
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
        if self.count == 0 { None } else { Some(&self.entries[self.head]) }
    }

    /// Returns a mutable reference to the head entry.
    pub fn peek_head_mut(&mut self) -> Option<&mut RobEntry> {
        if self.count == 0 { None } else { Some(&mut self.entries[self.head]) }
    }

    /// Commits (retires) the head entry. Returns the entry if it was Completed or Faulted.
    /// Returns `None` if the ROB is empty or the head is still Issued.
    pub fn commit_head(&mut self) -> Option<RobEntry> {
        if self.count == 0 {
            return None;
        }

        let entry = &self.entries[self.head];
        if entry.state == RobState::Issued {
            return None;
        }

        let committed = self.entries[self.head].clone();
        let _ = self.tag_index.remove(&committed.tag);
        if committed.vec_csr_update.is_some() {
            self.vec_config_updates -= 1;
        }
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
        self.tag_index.clear();
        self.head = 0;
        self.tail = 0;
        self.count = 0;
        self.vec_config_updates = 0;
    }

    /// Flushes all entries allocated *after* the given tag (exclusive).
    /// The entry with `tag` itself is kept.
    pub fn flush_after(&mut self, tag: RobTag) {
        if self.count == 0 {
            return;
        }

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

        let keep_idx = (idx + 1) % self.entries.len();

        // Avoid recount bug when ROB is full: head == tail means both full and empty.
        if keep_idx == self.tail {
            return;
        }

        let mut remove_idx = keep_idx;
        while remove_idx != self.tail {
            let _ = self.tag_index.remove(&self.entries[remove_idx].tag);
            if self.entries[remove_idx].vec_csr_update.is_some() {
                self.vec_config_updates -= 1;
            }
            self.entries[remove_idx].valid = false;
            remove_idx = (remove_idx + 1) % self.entries.len();
        }

        self.tail = keep_idx;
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
    pub fn find_latest_result(&self, reg: RegIdx, is_fp: bool) -> Option<u64> {
        if self.count == 0 || (!is_fp && reg.is_zero()) {
            return None;
        }

        let mut idx = if self.tail == 0 { self.entries.len() - 1 } else { self.tail - 1 };

        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.rd == reg && entry.rd_fp == is_fp {
                if entry.state == RobState::Completed {
                    return entry.result;
                }
                // Producer not ready — caller must stall or use bypass.
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
    /// Returns `Some((value, is_ready))` where `is_ready` indicates if the value is available.
    pub fn find_latest_producer(&self, reg: RegIdx, is_fp: bool) -> Option<(u64, bool)> {
        if self.count == 0 || (!is_fp && reg.is_zero()) {
            return None;
        }

        let mut idx = if self.tail == 0 { self.entries.len() - 1 } else { self.tail - 1 };

        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.rd == reg && entry.rd_fp == is_fp {
                let writes = if is_fp { entry.ctrl.fp_reg_write } else { entry.ctrl.reg_write };
                if writes {
                    let ready = entry.state == RobState::Completed;
                    return Some((entry.result.unwrap_or(0), ready));
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

    /// Returns the tag of the ROB entry immediately before `tag` in program
    /// order, or `None` if `tag` is at the head (no preceding in-flight entry).
    ///
    /// This walks the ROB from head to tail and returns the last entry seen
    /// before hitting `tag`. Unlike synthesizing `RobTag(tag.0 - 1)`, this
    /// always returns a tag that is actually present in the ROB.
    pub fn prev_tag_of(&self, tag: RobTag) -> Option<RobTag> {
        if self.count == 0 {
            return None;
        }
        let mut prev: Option<RobTag> = None;
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid {
                if entry.tag == tag {
                    return prev;
                }
                prev = Some(entry.tag);
            }
            idx = (idx + 1) % self.entries.len();
        }
        None
    }

    /// Finds a mutable reference to the entry with the given tag.
    fn find_entry_mut(&mut self, tag: RobTag) -> Option<&mut RobEntry> {
        let idx = *self.tag_index.get(&tag)?;
        let entry = &mut self.entries[idx];
        if entry.valid { Some(entry) } else { None }
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
        let idx = *self.tag_index.get(&tag)?;
        let entry = &self.entries[idx];
        if entry.valid { Some(entry) } else { None }
    }

    /// Iterate over all valid entries from head to tail in program order.
    pub fn iter_in_order(&self) -> impl Iterator<Item = &RobEntry> {
        let cap = self.entries.len();
        let head = self.head;
        let count = self.count;
        (0..count).filter_map(move |i| {
            let idx = (head + i) % cap;
            let e = &self.entries[idx];
            if e.valid { Some(e) } else { None }
        })
    }

    /// Iterate over all valid entries with `tag > keep_tag` (i.e., entries that
    /// would be squashed by `flush_after(keep_tag)`).
    pub fn iter_after(&self, keep_tag: RobTag) -> impl Iterator<Item = &RobEntry> {
        let cap = self.entries.len();
        let head = self.head;
        let count = self.count;
        let entries_ptr = self.entries.as_ptr();
        (0..count).filter_map(move |i| {
            let idx = (head + i) % cap;
            // SAFETY: idx is always in bounds (< cap), and we hold a shared ref to Rob.
            let e = unsafe { &*entries_ptr.add(idx) };
            if e.valid && e.tag.is_newer_than(keep_tag) { Some(e) } else { None }
        })
    }

    /// Iterate over all valid entries (head to tail).
    pub fn iter_all(&self) -> impl Iterator<Item = &RobEntry> {
        let cap = self.entries.len();
        let head = self.head;
        let count = self.count;
        let entries_ptr = self.entries.as_ptr();
        (0..count).filter_map(move |i| {
            let idx = (head + i) % cap;
            // SAFETY: idx is always in bounds (< cap), and we hold a shared ref to Rob.
            let e = unsafe { &*entries_ptr.add(idx) };
            if e.valid { Some(e) } else { None }
        })
    }

    /// Returns true if all ROB entries older than `tag` are Completed or Faulted.
    ///
    /// Used by the issue queue to enforce serializing behavior: system/CSR
    /// instructions must not issue until all older instructions have finished
    /// executing (e.g., FP instructions that set fflags).
    pub fn all_before_completed(&self, tag: RobTag) -> bool {
        if self.count == 0 {
            return true;
        }
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid {
                if entry.tag == tag {
                    return true;
                }
                if entry.state == RobState::Issued {
                    return false;
                }
            }
            idx = (idx + 1) % self.entries.len();
        }
        true
    }

    /// Returns true if all older ROB entries matching a FENCE's predecessor
    /// set have completed (Completed or Faulted).
    ///
    /// `pred_r` = true means older loads must have completed.
    /// `pred_w` = true means older stores must have completed.
    /// If neither is set, the FENCE is vacuously ready.
    pub fn fence_pred_satisfied(&self, tag: RobTag, pred_r: bool, pred_w: bool) -> bool {
        if !pred_r && !pred_w {
            return true;
        }
        if self.count == 0 {
            return true;
        }
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid {
                if entry.tag == tag {
                    return true;
                }
                if entry.state == RobState::Issued {
                    let dominated = (pred_r && entry.ctrl.reads_memory())
                        || (pred_w && entry.ctrl.writes_memory());
                    if dominated {
                        return false;
                    }
                }
            }
            idx = (idx + 1) % self.entries.len();
        }
        true
    }

    /// Checks if an older in-flight FENCE in the ROB blocks issuance of an
    /// instruction with the given `tag`, `is_load`, and `is_store` flags.
    ///
    /// A FENCE with successor bits `succ.r` / `succ.w` prevents younger
    /// loads/stores (respectively) from issuing until the FENCE has committed.
    /// An atomic with the `aq` bit must perform before anything after it, so
    /// it holds back younger loads until it has completed (gem5 splits an
    /// `aq` atomic into the atomic and a full barrier). Returns `true` if
    /// the instruction is blocked.
    pub fn has_fence_blocking(&self, tag: RobTag, is_load: bool, is_store: bool) -> bool {
        if self.count == 0 || (!is_load && !is_store) {
            return false;
        }
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid {
                if entry.tag == tag {
                    return false;
                }
                if is_load && entry.ctrl.acquire && entry.state == RobState::Issued {
                    return true;
                }
                if entry.ctrl.system_op == crate::core::exec::signals::SystemOp::Fence {
                    let succ_bits = ((entry.inst >> 20) & 0xF) as u8;
                    let succ_r = succ_bits & 0b0010 != 0;
                    let succ_w = succ_bits & 0b0001 != 0;
                    let blocked = (is_load && succ_r) || (is_store && succ_w);
                    if blocked {
                        return true;
                    }
                }
            }
            idx = (idx + 1) % self.entries.len();
        }
        false
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::{CsrAddr, RegIdx};
    use crate::core::exec::signals::ControlSignals;
    use crate::core::pipeline::prf::PhysReg;

    fn make_ctrl(reg_write: bool, fp_reg_write: bool) -> ControlSignals {
        ControlSignals { reg_write, fp_reg_write, ..Default::default() }
    }

    fn alloc(rob: &mut Rob, pc: u64, rd: u8, ctrl: ControlSignals) -> Option<RobTag> {
        rob.allocate(
            pc,
            0,
            InstSize::Standard,
            RegIdx::new(rd),
            false,
            ctrl,
            PhysReg(0),
            PhysReg(0),
            crate::common::InstSeq::default(),
        )
    }

    #[test]
    fn test_allocate_and_commit() {
        let mut rob = Rob::new(4);
        assert!(rob.is_empty());
        assert_eq!(rob.free_slots(), 4);

        let tag = rob
            .allocate(
                0x1000,
                0x13,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                make_ctrl(true, false),
                PhysReg(0),
                PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap();
        assert_eq!(rob.len(), 1);
        assert_eq!(rob.free_slots(), 3);

        // Can't commit while still Issued
        assert!(rob.commit_head().is_none());

        rob.complete(tag, 42);
        let entry = rob.commit_head().unwrap();
        assert_eq!(entry.pc, 0x1000);
        assert_eq!(entry.result, Some(42));
        assert_eq!(entry.state, RobState::Completed);
        assert!(rob.is_empty());
    }

    #[test]
    fn test_full_rob() {
        let mut rob = Rob::new(2);
        let _t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
        let _t2 = alloc(&mut rob, 0x1004, 2, make_ctrl(true, false)).unwrap();
        assert!(rob.is_full());
        assert!(alloc(&mut rob, 0x1008, 3, make_ctrl(true, false)).is_none());
    }

    #[test]
    fn test_in_order_commit() {
        let mut rob = Rob::new(4);
        let t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
        let t2 = alloc(&mut rob, 0x1004, 2, make_ctrl(true, false)).unwrap();

        // Complete t2 first (out of order)
        rob.complete(t2, 200);
        // t1 is still Issued, so commit should fail
        assert!(rob.commit_head().is_none());

        // Now complete t1
        rob.complete(t1, 100);
        let e1 = rob.commit_head().unwrap();
        assert_eq!(e1.result, Some(100));

        let e2 = rob.commit_head().unwrap();
        assert_eq!(e2.result, Some(200));
    }

    #[test]
    fn test_fault_commit() {
        let mut rob = Rob::new(4);
        let t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
        rob.fault(t1, Trap::IllegalInstruction(0), ExceptionStage::Decode);

        let entry = rob.commit_head().unwrap();
        assert_eq!(entry.state, RobState::Faulted);
        assert!(entry.trap.is_some());
    }

    #[test]
    fn test_flush_all() {
        let mut rob = Rob::new(4);
        alloc(&mut rob, 0x1000, 1, make_ctrl(true, false));
        alloc(&mut rob, 0x1004, 2, make_ctrl(true, false));
        assert_eq!(rob.len(), 2);

        rob.flush_all();
        assert!(rob.is_empty());
        assert_eq!(rob.free_slots(), 4);
    }

    #[test]
    fn test_flush_after() {
        let mut rob = Rob::new(8);
        let t1 = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
        let _t2 = alloc(&mut rob, 0x1004, 2, make_ctrl(true, false)).unwrap();
        let _t3 = alloc(&mut rob, 0x1008, 3, make_ctrl(true, false)).unwrap();
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
        let t1 = alloc(&mut rob, 0x1000, 5, make_ctrl(true, false)).unwrap();
        let t2 = alloc(&mut rob, 0x1004, 5, make_ctrl(true, false)).unwrap();

        rob.complete(t1, 100);
        rob.complete(t2, 200);

        // Should find t2's result (most recent)
        assert_eq!(rob.find_latest_result(RegIdx::new(5), false), Some(200));
        // x0 always returns None
        assert_eq!(rob.find_latest_result(RegIdx::new(0), false), None);
        // Non-existent register
        assert_eq!(rob.find_latest_result(RegIdx::new(10), false), None);
    }

    #[test]
    fn test_find_latest_result_not_ready() {
        let mut rob = Rob::new(8);
        alloc(&mut rob, 0x1000, 5, make_ctrl(true, false));
        // Entry is still Issued, so result is not ready
        assert_eq!(rob.find_latest_result(RegIdx::new(5), false), None);
    }

    #[test]
    fn test_csr_update() {
        let mut rob = Rob::new(4);
        let tag = alloc(&mut rob, 0x1000, 1, make_ctrl(true, false)).unwrap();
        rob.set_csr_update(
            tag,
            CsrUpdate { addr: CsrAddr::from_u32(0x300), old_val: 10, new_val: 20, applied: false },
        );
        rob.complete(tag, 10);

        let entry = rob.commit_head().unwrap();
        let csr = entry.csr_update.unwrap();
        assert_eq!(csr.addr, CsrAddr::from_u32(0x300));
        assert_eq!(csr.new_val, 20);
    }

    #[test]
    fn test_circular_wraparound() {
        let mut rob = Rob::new(2);

        // Fill and drain several times to test wraparound
        for i in 0..10 {
            let tag = alloc(&mut rob, i * 4, 1, make_ctrl(true, false)).unwrap();
            rob.complete(tag, i);
            let entry = rob.commit_head().unwrap();
            assert_eq!(entry.result, Some(i));
        }
    }

    /// Encode a FENCE instruction with given pred/succ bits.
    /// FENCE encoding: opcode=0x0F, funct3=0, pred in bits[27:24], succ in bits[23:20].
    fn encode_fence(pred: u8, succ: u8) -> u32 {
        0x0F | ((pred as u32 & 0xF) << 24) | ((succ as u32 & 0xF) << 20)
    }

    fn alloc_with_inst(rob: &mut Rob, inst: u32, ctrl: ControlSignals) -> Option<RobTag> {
        rob.allocate(
            0x1000,
            inst,
            InstSize::Standard,
            RegIdx::new(0),
            false,
            ctrl,
            PhysReg(0),
            PhysReg(0),
            crate::common::InstSeq::default(),
        )
    }

    #[test]
    fn test_fence_pred_satisfied() {
        let mut rob = Rob::new(8);

        // Allocate: store (tag1), load (tag2), FENCE rw,rw (tag3)
        let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };
        let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
        let fence_ctrl = ControlSignals {
            system_op: crate::core::exec::signals::SystemOp::Fence,
            ..Default::default()
        };

        let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();
        let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
        // FENCE rw,rw: pred=0b0011, succ=0b0011
        let t_fence = alloc_with_inst(&mut rob, encode_fence(0b0011, 0b0011), fence_ctrl).unwrap();

        // pred.r=true, pred.w=true: both older load and store must complete
        assert!(!rob.fence_pred_satisfied(t_fence, true, true));

        // Complete the store — still blocked by uncompleted load (pred.r)
        rob.complete(t_store, 0);
        assert!(!rob.fence_pred_satisfied(t_fence, true, true));

        // But pred.w only (FENCE w,*) would be satisfied now
        assert!(rob.fence_pred_satisfied(t_fence, false, true));

        // Complete the load — now fully satisfied
        rob.complete(t_load, 0);
        assert!(rob.fence_pred_satisfied(t_fence, true, true));
    }

    #[test]
    fn test_has_fence_blocking() {
        let mut rob = Rob::new(8);

        // Allocate: FENCE w,r (tag1), load (tag2), store (tag3)
        let fence_ctrl = ControlSignals {
            system_op: crate::core::exec::signals::SystemOp::Fence,
            ..Default::default()
        };
        let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
        let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };

        // FENCE w,r: pred=0b0001 (W), succ=0b0010 (R)
        let _t_fence = alloc_with_inst(&mut rob, encode_fence(0b0001, 0b0010), fence_ctrl).unwrap();
        let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
        let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();

        // Load is blocked (succ.r = true)
        assert!(rob.has_fence_blocking(t_load, true, false));
        // Store is NOT blocked (succ.w = false)
        assert!(!rob.has_fence_blocking(t_store, false, true));
    }

    #[test]
    fn an_acquire_atomic_holds_younger_loads_until_it_completes() {
        let mut rob = Rob::new(8);
        let acquire_ctrl = ControlSignals {
            atomic_op: crate::core::exec::signals::AtomicOp::Swap,
            acquire: true,
            mem_read: true,
            mem_write: true,
            ..Default::default()
        };
        let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
        let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };
        let t_amo = alloc_with_inst(&mut rob, 0, acquire_ctrl).unwrap();
        let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
        let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();

        assert!(rob.has_fence_blocking(t_load, true, false));
        assert!(!rob.has_fence_blocking(t_store, false, true));
        rob.complete(t_amo, 0);
        assert!(!rob.has_fence_blocking(t_load, true, false));
    }

    #[test]
    fn a_fence_waits_for_an_older_vector_store() {
        let mut rob = Rob::new(8);
        let vector_store = ControlSignals {
            vec_op: crate::core::exec::signals::VectorOp::VStoreUnit,
            ..Default::default()
        };
        let fence_ctrl = ControlSignals {
            system_op: crate::core::exec::signals::SystemOp::Fence,
            ..Default::default()
        };
        let t_store = alloc_with_inst(&mut rob, 0, vector_store).unwrap();
        let t_fence = alloc_with_inst(&mut rob, encode_fence(0b0001, 0b0001), fence_ctrl).unwrap();

        assert!(!rob.fence_pred_satisfied(t_fence, false, true));
        rob.complete(t_store, 0);
        assert!(rob.fence_pred_satisfied(t_fence, false, true));
    }

    #[test]
    fn test_fence_tso_blocking() {
        let mut rob = Rob::new(8);

        // FENCE.TSO = FENCE rw,rw
        let fence_ctrl = ControlSignals {
            system_op: crate::core::exec::signals::SystemOp::Fence,
            ..Default::default()
        };
        let load_ctrl = ControlSignals { mem_read: true, ..Default::default() };
        let store_ctrl = ControlSignals { mem_write: true, ..Default::default() };

        // FENCE rw,rw: pred=0b0011, succ=0b0011
        let _t_fence = alloc_with_inst(&mut rob, encode_fence(0b0011, 0b0011), fence_ctrl).unwrap();
        let t_load = alloc_with_inst(&mut rob, 0, load_ctrl).unwrap();
        let t_store = alloc_with_inst(&mut rob, 0, store_ctrl).unwrap();

        // Both loads and stores are blocked
        assert!(rob.has_fence_blocking(t_load, true, false));
        assert!(rob.has_fence_blocking(t_store, false, true));
    }

    #[test]
    fn test_control_outcome() {
        let mut rob = Rob::new(4);
        let t1 = alloc_with_inst(&mut rob, 0, ControlSignals::default()).unwrap();
        rob.set_control_outcome(t1, BpOutcome { taken: true, mispredicted: false }, Some(0x2000));

        let entry = rob.find_entry(t1).unwrap();
        assert!(entry.control_resolved);
        assert!(entry.bp_outcome.taken);
        assert_eq!(entry.bp_target, Some(0x2000));
        assert!(!entry.bp_outcome.mispredicted);
    }
}
