//! Reorder Buffer (ROB) for out-of-order commit.
//!
//! The ROB is a circular buffer that tracks in-flight instructions from rename
//! through commit. It provides:
//! 1. **Allocation:** Assigns unique tags to instructions entering the backend.
//! 2. **Completion:** Marks instructions as done when their results are available.
//! 3. **In-order Commit:** Retires instructions from the head in program order.
//! 4. **Forwarding:** Provides the most recent result for any register from in-flight instructions.
//! 5. **Flush:** Squashes speculative entries after a misprediction or trap.

use crate::sim::memory::write_log::WriteSeq;
use std::collections::HashMap;

use crate::arch::reservation::LrScRecord;
use crate::arch::translation::{DirtyUpdates, SfenceVmaInfo};
use crate::common::InstSeq;
use crate::exec::compute::vector::mem::{is_vec_load, is_vec_store};
use crate::exec::compute::vector::shadow::{ElementWrite, VectorWrites};
use crate::exec::execute::{CsrRequest, CsrWrite};
use crate::exec::signals::ControlSignals;
use crate::isa::csr::CsrAddr;
use crate::isa::instruction::InstSize;
use crate::isa::privileged::Trap;
use crate::isa::reg::RegIdx;
use crate::isa::rvv::VectorConfig;
use crate::uarch::pipeline::exception::ExceptionStage;
use crate::uarch::pipeline::rename::checkpoint::CheckpointId;
use crate::uarch::pipeline::rename::prf::PhysReg;
use crate::uarch::pipeline::rename::vec_prf::VecPhysReg;

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
/// skipping 0), so their numbers do not order them: only the ROB allocates
/// one, and in-flight tags are ordered by [`RobTag::age_cmp`] /
/// [`RobTag::is_older_than`], which handle the wrap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct RobTag(u32);

impl std::fmt::Display for RobTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl RobTag {
    /// A tag with the given number, for tests that build pipeline state.
    #[cfg(test)]
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The tag's number, for reports outside the pipeline. It says nothing
    /// about age across the wrap.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

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

/// The oldest instruction as a cycle began, before that cycle's commit. An
/// instruction that must be the oldest checks this rather than the live
/// head: commit registers a retirement, so issue learns of it a cycle later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeadAtCycleStart(Option<RobTag>);

impl HeadAtCycleStart {
    /// Latches `rob`'s head; called before the cycle's commit.
    #[must_use]
    pub fn latch(rob: &Rob) -> Self {
        Self(rob.peek_head().map(|head| head.tag))
    }

    /// True when `tag` was the oldest instruction as the cycle began.
    #[must_use]
    pub fn is(self, tag: RobTag) -> bool {
        self.0 == Some(tag)
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
    /// Computed result value (ALU output, load data, or link address).
    /// `None` while the instruction is still executing (`Issued` state);
    /// `Some(value)` once the instruction completes.
    pub result: Option<u64>,
    /// Control signals from decode.
    pub ctrl: ControlSignals,
    /// Current lifecycle state.
    pub state: RobState,
    /// Trap associated with this instruction, if faulted.
    pub trap: Option<Trap>,
    /// Pipeline stage where the exception was first detected.
    pub exception_stage: Option<ExceptionStage>,
    /// A CSR access the in-order backend performs when the entry reaches
    /// the head of the ROB; the entry has no result until then.
    pub csr_request: Option<CsrRequest>,
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
    /// The access a load, LR, SC or AMO performed, for the commit log.
    #[cfg(feature = "commit-log")]
    pub mem_effect: Option<crate::uarch::pipeline::commit_log::MemEffect>,
    /// The destination bits a vector instruction filled under an agnostic
    /// policy, for the commit log.
    #[cfg(feature = "commit-log")]
    pub vec_agnostic: Option<Box<crate::exec::compute::vector::agnostic::AgnosticFills>>,
    /// The element accesses a vector memory instruction's micro-ops made,
    /// for the commit log.
    #[cfg(feature = "commit-log")]
    pub vec_mem_effects: Vec<crate::uarch::pipeline::commit_log::VectorElementAccess>,
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

    /// Makes this empty ROB allocate tags from `next` on, for tests that
    /// cross the wrap.
    #[cfg(test)]
    pub fn start_tags_at(&mut self, next: RobTag) {
        assert!(self.is_empty(), "tags restart only in an empty ROB");
        self.next_tag = next.0;
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
            result: None,
            ctrl,
            state: RobState::Issued,
            trap: None,
            exception_stage: None,
            csr_request: None,
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
            #[cfg(feature = "commit-log")]
            mem_effect: None,
            #[cfg(feature = "commit-log")]
            vec_agnostic: None,
            #[cfg(feature = "commit-log")]
            vec_mem_effects: Vec::new(),
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
            if entry.csr_request.is_none() {
                entry.result = Some(result);
            }
        }
    }

    /// Records an entry's result the cycle its unit produces it, before the
    /// entry reaches writeback: the bypass a dependent reads from.
    pub fn forward(&mut self, tag: RobTag, result: u64) {
        if let Some(entry) = self.find_entry_mut(tag)
            && entry.state == RobState::Issued
            && entry.csr_request.is_none()
        {
            entry.result = Some(result);
        }
    }

    /// Defers `tag`'s CSR access to its retirement, where every older CSR
    /// write has been applied; dependents wait for its result until then.
    pub fn defer_csr(&mut self, tag: RobTag, request: CsrRequest) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.csr_request = Some(request);
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

    /// Records the access a load, LR, SC or AMO performed, for the commit log.
    #[cfg(feature = "commit-log")]
    pub fn set_mem_effect(
        &mut self,
        tag: RobTag,
        effect: crate::uarch::pipeline::commit_log::MemEffect,
    ) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.mem_effect = Some(effect);
        }
    }

    /// Records the destination bits a vector instruction filled under an
    /// agnostic policy, for the commit log.
    #[cfg(feature = "commit-log")]
    pub fn set_vec_agnostic(
        &mut self,
        tag: RobTag,
        fills: crate::exec::compute::vector::agnostic::AgnosticFills,
    ) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.vec_agnostic = Some(Box::new(fills));
        }
    }

    /// Adds the element accesses a vector memory micro-op made, for the
    /// commit log.
    #[cfg(feature = "commit-log")]
    pub fn add_vec_mem_effects(
        &mut self,
        tag: RobTag,
        accesses: &[crate::uarch::pipeline::commit_log::VectorElementAccess],
    ) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.vec_mem_effects.extend_from_slice(accesses);
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

    /// Returns a reference to the head entry (oldest), if the ROB is non-empty.
    pub fn peek_head(&self) -> Option<&RobEntry> {
        if self.count == 0 { None } else { Some(&self.entries[self.head]) }
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

    /// Returns the tag of the ROB entry immediately before `tag` in program
    /// order, or `None` if `tag` is at the head (no preceding in-flight entry).
    ///
    /// This walks the ROB from head to tail and returns the last entry seen
    /// before hitting `tag`. Unlike synthesizing a tag one below `tag`, this
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
        self.iter_all().filter(move |e| e.tag.is_newer_than(keep_tag))
    }

    /// Iterate over all valid entries (head to tail).
    pub fn iter_all(&self) -> impl Iterator<Item = &RobEntry> {
        let entries = &self.entries;
        let head = self.head;
        (0..self.count).map(move |i| &entries[(head + i) % entries.len()]).filter(|e| e.valid)
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

    /// Whether an instruction older than `tag` that reads or writes memory,
    /// a vector or cache-block one included, is still in flight.
    pub fn has_older_memory_access(&self, tag: RobTag) -> bool {
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid {
                if entry.tag == tag {
                    return false;
                }
                if entry.ctrl.mem_read
                    || entry.ctrl.mem_write
                    || is_vec_load(entry.ctrl.vec_op)
                    || is_vec_store(entry.ctrl.vec_op)
                    || entry.ctrl.system_op.is_cbo()
                {
                    return true;
                }
            }
            idx = (idx + 1) % self.entries.len();
        }
        false
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
                if entry.ctrl.system_op == crate::isa::op::SystemOp::Fence {
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
mod tests;
