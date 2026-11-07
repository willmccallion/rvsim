//! Reorder Buffer (ROB) for out-of-order commit.
//!
//! The ROB is a circular buffer that tracks in-flight instructions from rename
//! through commit. It provides:
//! 1. **Allocation:** Assigns unique tags to instructions entering the backend.
//! 2. **Completion:** Marks instructions as done when their results are available.
//! 3. **In-order Commit:** Retires instructions from the head in program order.
//! 4. **Forwarding:** Provides the most recent result for any register from in-flight instructions.
//! 5. **Flush:** Squashes speculative entries after a misprediction or trap.

#[cfg(feature = "commit-log")]
use crate::exec::compute::vector::agnostic::AgnosticFills;
use crate::isa::op::SystemOp;
use crate::sim::memory::write_log::WriteSeq;
#[cfg(feature = "commit-log")]
use crate::uarch::pipeline::commit_log::MemEffect;
#[cfg(feature = "commit-log")]
use crate::uarch::pipeline::commit_log::VectorElementAccess;
use crate::uarch::pipeline::latches::MemAccess;
use crate::uarch::pipeline::squash::SquashScope;
use std::collections::VecDeque;

use crate::arch::reservation::LrScRecord;
use crate::arch::translation::{DirtyUpdates, SfenceVmaInfo};
use crate::common::{InstSeq, VirtAddr};
use crate::exec::compute::vector::mem::{is_vec_load, is_vec_store};
use crate::exec::compute::vector::shadow::{ElementWrite, VectorWrites};
use crate::exec::execute::{CsrRequest, CsrWrite};
use crate::exec::inst::Inst;
use crate::exec::signals::ControlSignals;
use crate::isa::csr::CsrAddr;
use crate::isa::fence::{Fence, FenceSet};
use crate::isa::fp::FpFlags;
use crate::isa::instruction::InstSize;
use crate::isa::privileged::Trap;
use crate::isa::reg::RegIdx;
use crate::isa::rvv::VectorConfig;
use crate::uarch::pipeline::rename::checkpoint::CheckpointId;
use crate::uarch::pipeline::rename::prf::PhysReg;
use crate::uarch::pipeline::rename::vec_prf::VecRegGroup;

/// Branch outcome recorded at execute time for deferred predictor update.
///
/// Grouping `taken` and `mispredicted` into a struct prevents accidentally
/// swapping the two adjacent bools at call sites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BpOutcome {
    /// Whether the branch was actually taken.
    pub taken: bool,
    /// Whether the branch was mispredicted (i.e. fetch used the wrong path).
    pub mispredicted: bool,
}

/// The physical register rename gave an instruction's destination, and
/// the one the destination mapped to before, which commit frees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenamedDst {
    /// The new register.
    pub phys: PhysReg,
    /// The register it replaces.
    pub old: PhysReg,
}

impl RenamedDst {
    /// Nothing renamed: no destination, or a backend that renames nothing.
    /// x0's register takes the result, which discards it, and commit frees
    /// nothing.
    pub const NONE: Self = Self { phys: PhysReg::ZERO, old: PhysReg::ZERO };
}

/// How a branch or jump resolved at execute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedControl {
    /// Whether it was taken, and whether fetch predicted otherwise.
    pub outcome: BpOutcome,
    /// Where it went when taken; `None` for a branch not taken.
    pub target: Option<VirtAddr>,
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
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum RobState {
    /// Entry allocated but instruction not yet finished executing.
    #[default]
    Issued,
    /// Execution complete, result available, waiting to commit.
    Completed,
    /// Instruction faulted; the trap is taken when it reaches ROB head.
    Faulted(Trap),
}

impl RobState {
    /// Whether the instruction faulted.
    #[must_use]
    pub const fn is_faulted(&self) -> bool {
        matches!(self, Self::Faulted(_))
    }
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
}

impl From<CsrWrite> for CsrUpdate {
    fn from(write: CsrWrite) -> Self {
        Self { addr: write.addr, old_val: write.old, new_val: write.new }
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
    pub pc: VirtAddr,
    /// Its encoding, expanded to 32 bits when it is compressed.
    pub bits: u32,
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
    /// The physical register rename gave rd, and the one rd had before.
    pub dst: RenamedDst,
    /// FP exception flags generated by this instruction (deferred to commit).
    pub fp_flags: FpFlags,
    /// How a branch or jump resolved, once it has; commit counts its
    /// prediction and goes where it went.
    pub control: Option<ResolvedControl>,
    /// The D-bit updates the access applies when it retires.
    pub dirty_updates: DirtyUpdates,
    /// Deferred SFENCE.VMA operands for commit-time TLB invalidation.
    pub sfence_vma: Option<SfenceVmaInfo>,
    /// Deferred LR/SC reservation action for commit-time application.
    pub lr_sc: Option<LrScRecord>,
    /// Write-log position when a load, LR or AMO read its value from RAM;
    /// commit re-validates LR and AMO against it.
    pub observed: Option<WriteSeq>,
    /// The address a load, store or atomic touched, for stops on a watched
    /// range.
    pub mem_access: Option<MemAccess>,
    /// Checkpoint table slot allocated for this branch/jump (O3 backend).
    pub checkpoint_id: Option<CheckpointId>,
    /// Physical vector registers rename gave the destination group (O3
    /// backend); empty when no vector register is renamed.
    pub vec_dst: VecRegGroup,
    /// The registers the destination group mapped to before, freed at
    /// commit.
    pub vec_old_dst: VecRegGroup,
    /// Deferred vxsat (fixed-point saturation) flag from vector execution (applied at commit).
    pub vxsat: bool,
    /// Vector register writes a backend without vector renaming holds
    /// back for commit; `Some` marks the entry as an executed vector op.
    pub vec_writes: Option<Box<VectorWrites>>,
    /// The access a load, LR, SC or AMO performed, for the commit log.
    #[cfg(feature = "commit-log")]
    pub mem_effect: Option<MemEffect>,
    /// The destination bits a vector instruction filled under an agnostic
    /// policy, for the commit log.
    #[cfg(feature = "commit-log")]
    pub vec_agnostic: Option<Box<AgnosticFills>>,
    /// The element accesses a vector memory instruction's micro-ops made,
    /// for the commit log.
    #[cfg(feature = "commit-log")]
    pub vec_mem_effects: Vec<VectorElementAccess>,
}

impl RobEntry {
    /// The trap the instruction raised, if it faulted.
    #[must_use]
    pub const fn trap(&self) -> Option<&Trap> {
        match &self.state {
            RobState::Faulted(trap) => Some(trap),
            RobState::Issued | RobState::Completed => None,
        }
    }

    /// The address of the instruction after this one.
    pub const fn next_pc(&self) -> VirtAddr {
        self.pc.wrapping_add(self.inst_size.as_u64())
    }

    /// True when the entry's result exists only once it commits: a CSR
    /// access the in-order backend deferred, a syscall (an `ecall` with
    /// `a0` as its destination in SE mode) or a pseudo-operation that
    /// returns a value.
    #[must_use]
    pub const fn result_at_commit(&self) -> bool {
        use crate::isa::op::SystemOp;

        self.csr_request.is_some()
            || (matches!(self.ctrl.system_op, SystemOp::Ecall | SystemOp::Pseudo)
                && self.ctrl.reg_write)
    }
}

/// Reorder Buffer: the in-flight instructions in program order, committed
/// from the oldest.
#[derive(Debug)]
pub struct Rob {
    /// The in-flight instructions, oldest first, so in tag age order.
    entries: VecDeque<RobEntry>,
    /// The most instructions it holds.
    capacity: usize,
    /// Monotonically increasing tag counter.
    next_tag: u32,
    /// Entries carrying a `vec_csr_update`.
    vec_config_updates: usize,
}

impl Rob {
    /// Creates a new ROB with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
            next_tag: 1,
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
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the ROB is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns true if the ROB is full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.entries.len() == self.capacity
    }

    /// Returns the number of free slots.
    #[inline]
    pub fn free_slots(&self) -> usize {
        self.capacity - self.entries.len()
    }

    /// Allocates an entry for `inst`, its destination renamed as `dst`.
    /// Returns `None` if the ROB is full.
    pub fn allocate(&mut self, inst: &Inst, seq: InstSeq, dst: RenamedDst) -> Option<RobTag> {
        if self.is_full() {
            return None;
        }

        let tag = RobTag(self.next_tag);
        self.next_tag = self.next_tag.wrapping_add(1);
        if self.next_tag == 0 {
            self.next_tag = 1;
        }

        self.entries.push_back(RobEntry {
            tag,
            seq,
            pc: inst.pc,
            bits: inst.bits,
            inst_size: inst.size,
            rd: inst.rd,
            result: None,
            ctrl: inst.ctrl,
            state: RobState::Issued,
            csr_request: None,
            csr_update: None,
            vec_csr_update: None,
            fault_vstart: None,
            vl_trim: None,
            dst,
            fp_flags: FpFlags::NONE,
            control: None,
            dirty_updates: DirtyUpdates::NONE,
            sfence_vma: None,
            lr_sc: None,
            observed: None,
            mem_access: None,
            checkpoint_id: None,
            vec_dst: VecRegGroup::EMPTY,
            vec_old_dst: VecRegGroup::EMPTY,
            vxsat: false,
            vec_writes: None,
            #[cfg(feature = "commit-log")]
            mem_effect: None,
            #[cfg(feature = "commit-log")]
            vec_agnostic: None,
            #[cfg(feature = "commit-log")]
            vec_mem_effects: Vec::new(),
        });
        Some(tag)
    }

    /// Marks an entry as Completed with its result value.
    ///
    /// Does nothing if the entry is already Faulted — a fault set during
    /// execute must not be overwritten by a later writeback completion.
    pub fn complete(&mut self, tag: RobTag, result: u64) {
        if let Some(entry) = self.find_entry_mut(tag)
            && !entry.state.is_faulted()
        {
            entry.state = RobState::Completed;
            if !entry.result_at_commit() {
                entry.result = Some(result);
            }
        }
    }

    /// Records an entry's result the cycle its unit produces it, before the
    /// entry reaches writeback: the bypass a dependent reads from.
    pub fn forward(&mut self, tag: RobTag, result: u64) {
        if let Some(entry) = self.find_entry_mut(tag)
            && entry.state == RobState::Issued
            && !entry.result_at_commit()
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
    pub fn fault(&mut self, tag: RobTag, trap: Trap) {
        if let Some(entry) = self.find_entry_mut(tag)
            && !entry.state.is_faulted()
        {
            entry.state = RobState::Faulted(trap);
        }
    }

    /// Faults a vector memory instruction at `element`, where its trap
    /// sets `vstart`.
    pub fn fault_element(&mut self, tag: RobTag, trap: Trap, element: u64) {
        if let Some(entry) = self.find_entry_mut(tag)
            && !entry.state.is_faulted()
        {
            entry.state = RobState::Faulted(trap);
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
        self.entries.iter().rev().find_map(|entry| entry.vec_csr_update)
    }

    /// Sets the CSR update for a given entry.
    pub fn set_csr_update(&mut self, tag: RobTag, update: CsrUpdate) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.csr_update = Some(update);
        }
    }

    /// Records a branch's or jump's resolved outcome for commit.
    pub fn set_control_outcome(
        &mut self,
        tag: RobTag,
        outcome: BpOutcome,
        target: Option<VirtAddr>,
    ) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.control = Some(ResolvedControl { outcome, target });
        }
    }

    /// Sets the FP exception flags for a given entry (accumulated at commit).
    pub fn set_fp_flags(&mut self, tag: RobTag, fp_flags: FpFlags) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.fp_flags = entry.fp_flags | fp_flags;
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

    /// Records the address a load, store or atomic touched.
    pub fn set_mem_access(&mut self, tag: RobTag, access: MemAccess) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.mem_access = Some(access);
        }
    }

    /// Records the access a load, LR, SC or AMO performed, for the commit log.
    #[cfg(feature = "commit-log")]
    pub fn set_mem_effect(&mut self, tag: RobTag, effect: MemEffect) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.mem_effect = Some(effect);
        }
    }

    /// Records the destination bits a vector instruction filled under an
    /// agnostic policy, for the commit log.
    #[cfg(feature = "commit-log")]
    pub fn set_vec_agnostic(&mut self, tag: RobTag, fills: AgnosticFills) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.vec_agnostic = Some(Box::new(fills));
        }
    }

    /// Adds the element accesses a vector memory micro-op made, for the
    /// commit log.
    #[cfg(feature = "commit-log")]
    pub fn add_vec_mem_effects(&mut self, tag: RobTag, accesses: &[VectorElementAccess]) {
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

    /// Records the registers rename gave `tag`'s destination group, and
    /// those the group mapped to before.
    pub fn set_vec_dst(&mut self, tag: RobTag, dst: VecRegGroup, old: VecRegGroup) {
        if let Some(entry) = self.find_entry_mut(tag) {
            entry.vec_dst = dst;
            entry.vec_old_dst = old;
        }
    }

    /// Returns a reference to the head entry (oldest), if the ROB is non-empty.
    pub fn peek_head(&self) -> Option<&RobEntry> {
        self.entries.front()
    }

    /// Commits (retires) the head entry. Returns the entry if it was Completed or Faulted.
    /// Returns `None` if the ROB is empty or the head is still Issued.
    pub fn commit_head(&mut self) -> Option<RobEntry> {
        if self.entries.front()?.state == RobState::Issued {
            return None;
        }
        let committed = self.entries.pop_front()?;
        if committed.vec_csr_update.is_some() {
            self.vec_config_updates -= 1;
        }
        Some(committed)
    }

    /// Removes what `scope` squashes.
    pub fn squash(&mut self, scope: SquashScope) {
        match scope {
            SquashScope::After(keep) => self.flush_after(keep),
            SquashScope::All => self.flush_all(),
        }
    }

    /// The entries `scope` squashes, oldest first.
    pub fn iter_squashed(&self, scope: SquashScope) -> impl Iterator<Item = &RobEntry> {
        self.iter().filter(move |entry| !scope.keeps(entry.tag))
    }

    /// Flushes all entries from the ROB.
    pub fn flush_all(&mut self) {
        self.entries.clear();
        self.vec_config_updates = 0;
    }

    /// Flushes all entries allocated *after* the given tag (exclusive).
    /// The entry with `tag` itself is kept.
    pub fn flush_after(&mut self, tag: RobTag) {
        let Some(keep) = self.position(tag) else { return };
        let squashed_updates =
            self.entries.range(keep + 1..).filter(|entry| entry.vec_csr_update.is_some()).count();
        self.vec_config_updates -= squashed_updates;
        self.entries.truncate(keep + 1);
    }

    /// Returns the tag of the ROB entry immediately before `tag` in program
    /// order, or `None` if `tag` is at the head (no preceding in-flight entry).
    pub fn prev_tag_of(&self, tag: RobTag) -> Option<RobTag> {
        let position = self.position(tag)?;
        Some(self.entries.get(position.checked_sub(1)?)?.tag)
    }

    /// Where `tag`'s entry sits, oldest first, if it is in flight.
    fn position(&self, tag: RobTag) -> Option<usize> {
        self.entries.binary_search_by(|entry| entry.tag.age_cmp(tag)).ok()
    }

    /// The entry `tag` names, or `None` once it has left the ROB. Units
    /// and memory still deliver results for squashed instructions, so
    /// every setter drops a write to a missing entry; only a tag that was
    /// never handed out is a bug.
    fn find_entry_mut(&mut self, tag: RobTag) -> Option<&mut RobEntry> {
        debug_assert!(tag.is_older_than(RobTag(self.next_tag)), "{tag} was never allocated");
        let position = self.position(tag)?;
        self.entries.get_mut(position)
    }

    /// Finds a reference to the entry with the given tag.
    pub fn find_entry(&self, tag: RobTag) -> Option<&RobEntry> {
        self.entries.get(self.position(tag)?)
    }

    /// The in-flight entries, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &RobEntry> {
        self.entries.iter()
    }

    /// The entries older than `tag`, oldest first: every entry when `tag`
    /// is not in flight.
    fn older_than(&self, tag: RobTag) -> impl Iterator<Item = &RobEntry> {
        self.entries.iter().take_while(move |entry| entry.tag != tag)
    }

    /// The sequence numbers of the entries a squash drops: those after
    /// `keep_tag`, or all of them.
    pub fn seqs_after(&self, keep_tag: Option<RobTag>) -> Vec<InstSeq> {
        self.iter()
            .filter(|entry| keep_tag.is_none_or(|keep| entry.tag.is_newer_than(keep)))
            .map(|entry| entry.seq)
            .collect()
    }

    /// Returns true if every entry older than `tag` making an access in
    /// a FENCE's predecessor set `pred` has completed (Completed or
    /// Faulted). A set naming no memory access is vacuously satisfied.
    pub fn fence_pred_satisfied(&self, tag: RobTag, pred: FenceSet) -> bool {
        !pred.has_memory()
            || !self.older_than(tag).any(|entry| {
                entry.state == RobState::Issued && pred.orders(entry.ctrl.memory_accesses())
            })
    }

    /// Whether an instruction older than `tag` that reads or writes memory,
    /// a vector or cache-block one included, is still in flight.
    pub fn has_older_memory_access(&self, tag: RobTag) -> bool {
        self.older_than(tag).any(|entry| {
            entry.ctrl.mem_read
                || entry.ctrl.mem_write
                || is_vec_load(entry.ctrl.vec_op)
                || is_vec_store(entry.ctrl.vec_op)
                || entry.ctrl.system_op.is_cbo()
        })
    }

    /// Checks if an older in-flight FENCE in the ROB blocks issuance of the
    /// instruction `tag`, which makes the memory `accesses`.
    ///
    /// A FENCE with successor bits `succ.r` / `succ.w` prevents younger
    /// loads/stores (respectively) from issuing until the FENCE has committed.
    /// An atomic with the `aq` bit must perform before anything after it, so
    /// it holds back younger loads until it has completed (gem5 splits an
    /// `aq` atomic into the atomic and a full barrier). Returns `true` if
    /// the instruction is blocked.
    pub fn has_fence_blocking(&self, tag: RobTag, accesses: FenceSet) -> bool {
        accesses.has_memory()
            && self.older_than(tag).any(|entry| {
                (accesses.r && entry.ctrl.acquire && entry.state == RobState::Issued)
                    || (entry.ctrl.system_op == SystemOp::Fence
                        && Fence::decode(entry.bits).succ.orders(accesses))
            })
    }
}

#[cfg(test)]
mod tests;
