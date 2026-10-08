//! CAM-style Issue Queue for the O3 backend.
//!
//! Instructions dispatched from rename sit in the issue queue until all source
//! operands are ready. The wakeup/select logic allows out-of-order issue:
//! - **Wakeup (PRF path)**: when an instruction completes, its `PhysReg` is broadcast
//!   to all waiting entries, marking matching source operands as ready.
//! - **Wakeup (legacy path)**: when an instruction completes, its ROB tag is broadcast.
//! - **Select**: each cycle, the oldest entries with all operands ready are
//!   selected for execution (up to `width`).

use crate::isa::op::{SystemOp, VectorOp};
use crate::isa::reg::RegIdx;
use crate::uarch::ctx::StageCtx;
use crate::uarch::mdp::MemDepState;
use crate::uarch::pipeline::backend::o3::fu_pool::{FU_TYPE_COUNT, FreeUnit, FuPool, FuType};
use crate::uarch::pipeline::backend::shared::issue_stats::IssueHold;
use crate::uarch::pipeline::latches::RenameIssueEntry;
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::rename::prf::{PhysReg, PhysRegFile};
use crate::uarch::pipeline::rename::vec_prf::VecPhysReg;
use crate::uarch::pipeline::rename::vec_prf::VecPhysRegFile;
use crate::uarch::pipeline::rob::{HeadAtCycleStart, Rob, RobState, RobTag};

/// Readiness state of a single source operand.
#[derive(Clone, Copy, Debug, Default)]
pub enum OperandReady {
    /// Value not available yet — waiting on producer.
    #[default]
    NotReady,
    /// Ready with a real value.
    Ready(u64),
}

impl OperandReady {
    /// Returns `true` if the operand is ready.
    #[inline]
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready(_))
    }
}

/// State of a single source operand in an issue queue entry.
#[derive(Clone, Debug, Default)]
pub struct OperandState {
    /// Which physical register provides this operand value.
    pub phys: PhysReg,
    /// Readiness and value of this operand.
    pub readiness: OperandReady,
}

impl OperandState {
    /// Convenience: create a ready operand with a known value.
    const fn ready(phys: PhysReg, value: u64) -> Self {
        Self { phys, readiness: OperandReady::Ready(value) }
    }

    /// Convenience: create a not-ready operand.
    const fn not_ready(phys: PhysReg) -> Self {
        Self { phys, readiness: OperandReady::NotReady }
    }
}

/// Readiness state of a vector source operand group (LMUL registers).
///
/// All physical registers in the group must be ready for the group to be ready.
/// Unlike scalar operands, vector values are read from Vec PRF at execute time,
/// not forwarded through the IQ.
#[derive(Clone, Debug)]
pub struct VecOperandState {
    /// Physical registers in this LMUL group.
    pub phys: [VecPhysReg; 8],
    /// Number of registers in this group.
    pub count: u8,
    /// True when ALL registers in the group are ready.
    pub ready: bool,
}

impl Default for VecOperandState {
    fn default() -> Self {
        Self { phys: [VecPhysReg::ZERO; 8], count: 0, ready: true }
    }
}

impl VecOperandState {
    /// Check if all physical registers in this group are ready in the Vec PRF.
    pub fn check_ready(&mut self, vec_prf: &VecPhysRegFile) {
        if self.count == 0 {
            self.ready = true;
            return;
        }
        self.ready = (0..self.count as usize).all(|i| vec_prf.is_ready(self.phys[i]));
    }

    /// Re-check readiness after a wakeup broadcast of physical register `p`.
    pub fn wakeup_check(&mut self, p: VecPhysReg, vec_prf: &VecPhysRegFile) {
        if self.ready || self.count == 0 {
            return;
        }
        let contains = (0..self.count as usize).any(|i| self.phys[i] == p);
        if contains {
            self.check_ready(vec_prf);
        }
    }
}

/// A single entry in the issue queue.
#[derive(Clone, Debug)]
pub struct IssueQueueEntry {
    /// The instruction from rename.
    pub entry: RenameIssueEntry,
    /// Source operand 1 state.
    pub src1: OperandState,
    /// Source operand 2 state.
    pub src2: OperandState,
    /// Source operand 3 state (FP fused multiply-add).
    pub src3: OperandState,
    /// Vector source 1 operand group state.
    pub vec_src1: VecOperandState,
    /// Vector source 2 operand group state.
    pub vec_src2: VecOperandState,
    /// Vector source 3 operand group state (vd-as-source for accumulating ops).
    pub vec_src3: VecOperandState,
    /// Cached memory dependency state (set once at dispatch).
    pub mem_dep: MemDepState,
    /// Physical register for v0 mask (tracked for masked vector ops).
    pub mask_phys: VecPhysReg,
    /// Whether the mask register v0 is ready.
    pub mask_ready: bool,
    /// Whether this instruction requires a ready mask register (vm=0 for vector ops).
    pub needs_mask: bool,
    /// How far a plain store's two halves have issued.
    pub store_issue: StoreIssue,
}

/// How far a plain store's address and data halves have issued. A store
/// issues its address as soon as its base register is ready, so younger
/// loads learn early whether it aliases them, and its data when the value
/// is ready; a store whose operands are both ready issues whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreIssue {
    /// Not a store that issues in halves.
    Whole,
    /// Neither half has issued.
    Unissued,
    /// The address half has issued; the data half waits for its value.
    AddressIssued,
}

/// The part of an entry that is ready to issue this cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IssuePart {
    /// The whole instruction.
    Whole,
    /// A store's address half.
    StoreAddress,
    /// A store's data half.
    StoreData,
}

impl IssueQueueEntry {
    /// Which part of the entry has the operands it needs this cycle.
    fn ready_part(&self) -> Option<IssuePart> {
        // Faulted instructions don't need operands — always ready.
        if self.entry.trap.is_some() {
            return Some(IssuePart::Whole);
        }
        let others_ready = self.src3.readiness.is_ready()
            && self.vec_src1.ready
            && self.vec_src2.ready
            && self.vec_src3.ready
            && self.mask_ready;
        let address = self.src1.readiness.is_ready() && others_ready;
        let data = self.src2.readiness.is_ready();
        match self.store_issue {
            StoreIssue::Whole => (address && data).then_some(IssuePart::Whole),
            StoreIssue::Unissued => {
                address.then_some(if data { IssuePart::Whole } else { IssuePart::StoreAddress })
            }
            StoreIssue::AddressIssued => data.then_some(IssuePart::StoreData),
        }
    }
}

/// An instruction selected from the IQ for execution.
///
/// Bundles the resolved `RenameIssueEntry` with its cached `MemDepState`
/// so that re-dispatch (on backpressure or FU stall) preserves the
/// original memory dependency prediction. Without this, re-dispatch
/// would lose the prediction and allow loads to bypass stores unsafely.
#[derive(Clone, Debug)]
pub struct SelectedEntry {
    /// The instruction with resolved operand values.
    pub entry: RenameIssueEntry,
    /// Its functional unit class.
    pub fu_type: FuType,
    /// The free unit reserved for it.
    pub unit: FreeUnit,
    /// It is a store's address half: its data half delivers the data.
    pub data_follows: bool,
}

/// A store's data half, which writes its value into the store's
/// store-buffer slot and takes no functional unit or store port.
#[derive(Clone, Copy, Debug)]
pub struct StoreDataIssue {
    /// The store.
    pub rob_tag: RobTag,
    /// The data register's value.
    pub value: u64,
}

/// What issue may use in one cycle.
#[derive(Clone, Copy, Debug)]
pub struct IssueBudget<'a> {
    /// Instructions issued per cycle.
    pub width: usize,
    /// Loads issued per cycle.
    pub load_ports: usize,
    /// Stores issued per cycle.
    pub store_ports: usize,
    /// The functional units, whose free ones the selected instructions take.
    pub units: &'a FuPool,
    /// The cycle.
    pub now: u64,
    /// The memory pipeline cannot take a memory op this cycle.
    pub memory_blocked: bool,
    /// The oldest instruction as the cycle began.
    pub head: HeadAtCycleStart,
}

/// The instructions [`IssueQueue::select`] chose, and how many ready ones
/// it passed over because every unit of their class was busy.
#[derive(Debug, Default)]
pub struct Selection {
    /// Chosen instructions, oldest first.
    pub entries: Vec<SelectedEntry>,
    /// Chosen store data halves.
    pub store_data: Vec<StoreDataIssue>,
    /// Ready instructions left waiting for a functional unit.
    pub unit_stalls: usize,
    /// What held the oldest instruction still queued: its operands, or the
    /// ordering rules; `None` when it was passed over for a unit or port.
    pub oldest: Option<IssueHold>,
}

/// CAM-style issue queue with wakeup and oldest-first select.
#[derive(Debug)]
pub struct IssueQueue {
    /// Fixed-size slot array. `None` = free slot.
    slots: Vec<Option<IssueQueueEntry>>,
    /// Maximum capacity.
    capacity: usize,
    /// Current number of occupied slots.
    count: usize,
}

impl IssueQueue {
    /// Create a new issue queue with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let mut slots = Vec::with_capacity(capacity);
        slots.resize_with(capacity, || None);
        Self { slots, capacity, count: 0 }
    }

    /// Dispatch an instruction from rename into the first free slot.
    ///
    /// For the O3 (PRF) path, resolves operands via the PRF.
    /// For the legacy (scoreboard) path, resolves operands via the ROB.
    pub fn dispatch(
        &mut self,
        entry: RenameIssueEntry,
        rob: &Rob,
        state: &StageCtx<'_>,
        prf: Option<&PhysRegFile>,
        vec_prf: Option<&VecPhysRegFile>,
        mem_dep: MemDepState,
    ) -> bool {
        let Some(free) = self.slots.iter().position(Option::is_none) else {
            return false;
        };

        let (src1, src2, src3) = if let Some(prf) = prf {
            let s1 = resolve_operand_prf(
                entry.inst.rs1,
                entry.inst.ctrl.rs1_fp,
                entry.rs1_phys,
                prf,
                state,
            );
            let s2 = resolve_operand_prf(
                entry.inst.rs2,
                entry.inst.ctrl.rs2_fp,
                entry.rs2_phys,
                prf,
                state,
            );
            let s3 = if entry.inst.ctrl.rs3_fp {
                resolve_operand_prf(entry.inst.rs3, true, entry.rs3_phys, prf, state)
            } else {
                OperandState::ready(PhysReg(0), 0)
            };
            (s1, s2, s3)
        } else {
            let s1 = resolve_operand_legacy(
                entry.inst.rs1,
                entry.inst.ctrl.rs1_fp,
                entry.rs1_tag,
                rob,
                state,
            );
            let s2 = resolve_operand_legacy(
                entry.inst.rs2,
                entry.inst.ctrl.rs2_fp,
                entry.rs2_tag,
                rob,
                state,
            );
            let s3 = if entry.inst.ctrl.rs3_fp {
                resolve_operand_legacy(entry.inst.rs3, true, entry.rs3_tag, rob, state)
            } else {
                OperandState::ready(PhysReg(0), 0)
            };
            (s1, s2, s3)
        };

        let mut vec_src1 = VecOperandState {
            phys: entry.vs1_phys,
            count: entry.vec_src1_count,
            ready: entry.vec_src1_count == 0,
        };
        let mut vec_src2 = VecOperandState {
            phys: entry.vs2_phys,
            count: entry.vec_src2_count,
            ready: entry.vec_src2_count == 0,
        };
        let mut vec_src3 = VecOperandState {
            phys: entry.vs3_phys,
            count: entry.vec_src3_count,
            ready: entry.vec_src3_count == 0,
        };

        if let Some(vprf) = vec_prf {
            vec_src1.check_ready(vprf);
            vec_src2.check_ready(vprf);
            vec_src3.check_ready(vprf);
        }

        // Track v0 mask register dependency for masked vector ops (vm=0).
        let needs_mask = !entry.inst.ctrl.vm
            && entry.inst.ctrl.vec_op != VectorOp::None
            && !entry.inst.ctrl.vec_op.is_config();
        let mask_phys = if needs_mask { entry.mask_phys } else { VecPhysReg::ZERO };
        let mask_ready = !needs_mask || vec_prf.is_none_or(|vprf| vprf.is_ready(mask_phys));

        let store_issue = if entry.inst.ctrl.splits_store() && entry.trap.is_none() {
            StoreIssue::Unissued
        } else {
            StoreIssue::Whole
        };
        let iq_entry = IssueQueueEntry {
            entry,
            src1,
            src2,
            src3,
            vec_src1,
            vec_src2,
            vec_src3,
            mem_dep,
            mask_phys,
            mask_ready,
            needs_mask,
            store_issue,
        };

        self.slots[free] = Some(iq_entry);
        self.count += 1;
        true
    }

    /// Broadcast a completed result via physical register (PRF wakeup path).
    pub fn wakeup_phys(&mut self, p: PhysReg, value: u64) {
        for iq in self.slots.iter_mut().flatten() {
            if iq.src1.phys == p && !iq.src1.readiness.is_ready() {
                iq.src1.readiness = OperandReady::Ready(value);
            }
            if iq.src2.phys == p && !iq.src2.readiness.is_ready() {
                iq.src2.readiness = OperandReady::Ready(value);
            }
            if iq.src3.phys == p && !iq.src3.readiness.is_ready() {
                iq.src3.readiness = OperandReady::Ready(value);
            }
        }
    }

    /// Broadcast a vector physical register wakeup to all waiting entries.
    ///
    /// Called when a vector destination register becomes ready (chaining wakeup).
    /// Re-checks each `VecOperandState` group that contains `p`.
    pub fn wakeup_vec_phys(&mut self, p: VecPhysReg, vec_prf: &VecPhysRegFile) {
        if p.is_zero() {
            return;
        }
        for iq in self.slots.iter_mut().flatten() {
            iq.vec_src1.wakeup_check(p, vec_prf);
            iq.vec_src2.wakeup_check(p, vec_prf);
            iq.vec_src3.wakeup_check(p, vec_prf);
            if iq.needs_mask && !iq.mask_ready && iq.mask_phys == p {
                iq.mask_ready = vec_prf.is_ready(p);
            }
        }
    }

    /// Selects up to `budget.width` ready entries, oldest first, as gem5's
    /// instruction scheduler does.
    ///
    /// Selected entries have their `rv1/rv2/rv3` fields populated from the
    /// resolved operand values and carry the free functional unit reserved
    /// for them. The slots are freed.
    ///
    /// Memory dependencies are checked via the cached [`MemDepState`] set at
    /// dispatch time, rather than re-querying the predictor every cycle.
    ///
    /// System/CSR instructions are serializing: they must not issue until all
    /// older ROB entries have completed.
    ///
    /// A ready entry is passed over, and a younger one considered in its
    /// place, when every unit of its class is busy, when it is a load or
    /// store beyond the cycle's load or store ports, or when it is a memory
    /// op while the memory pipeline is blocked.
    pub fn select(
        &mut self,
        budget: &IssueBudget<'_>,
        store_buffer: &StoreBuffer,
        rob: &Rob,
    ) -> Selection {
        let mut ready: Vec<(usize, IssuePart, RobTag)> = Vec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            let Some(iq) = slot else { continue };
            let Some(part) = iq.ready_part() else { continue };
            if part != IssuePart::StoreData
                && !Self::may_issue_now(iq, store_buffer, rob, budget.head)
            {
                continue;
            }
            ready.push((i, part, iq.entry.rob_tag));
        }

        ready.sort_by(|a, b| a.2.age_cmp(b.2));

        let mut selection = Selection {
            oldest: self.oldest_hold(store_buffer, rob, budget.head),
            ..Selection::default()
        };
        let mut loads_issued = 0usize;
        let mut stores_issued = 0usize;
        let mut units_taken = [0usize; FU_TYPE_COUNT];
        for &(idx, part, _) in &ready {
            if selection.entries.len() + selection.store_data.len() >= budget.width {
                break;
            }
            if part == IssuePart::StoreData {
                let Some(iq) = self.slots[idx].take() else { continue };
                self.count -= 1;
                let value = Self::resolve_value(&iq.src2);
                selection.store_data.push(StoreDataIssue { rob_tag: iq.entry.rob_tag, value });
                continue;
            }
            let Some(slot) = self.slots[idx].as_ref() else { continue };
            let ctrl = &slot.entry.inst.ctrl;
            let is_load = ctrl.mem_read;
            let is_store = ctrl.mem_write;
            if is_load && loads_issued >= budget.load_ports {
                continue;
            }
            if is_store && stores_issued >= budget.store_ports {
                continue;
            }
            let fu_type = FuType::classify(ctrl);
            if budget.memory_blocked && fu_type == FuType::Mem {
                continue;
            }
            let taken = &mut units_taken[fu_type as usize];
            let Some(unit) = budget.units.free_units(fu_type, budget.now).nth(*taken) else {
                selection.unit_stalls += 1;
                continue;
            };
            *taken += 1;
            if is_load {
                loads_issued += 1;
            }
            if is_store {
                stores_issued += 1;
            }

            let data_follows = part == IssuePart::StoreAddress;
            let entry = if data_follows {
                let Some(iq) = self.slots[idx].as_mut() else { continue };
                iq.store_issue = StoreIssue::AddressIssued;
                let mut entry = iq.entry.clone();
                entry.inst.rv1 = Self::resolve_value(&iq.src1);
                entry
            } else {
                let Some(iq) = self.slots[idx].take() else { continue };
                self.count -= 1;
                Self::with_operands(iq)
            };
            selection.entries.push(SelectedEntry { entry, fu_type, unit, data_follows });
        }

        selection
    }

    /// What holds the oldest queued instruction this cycle, before any is
    /// chosen: its operands, the ordering rules, or nothing.
    fn oldest_hold(
        &self,
        store_buffer: &StoreBuffer,
        rob: &Rob,
        head: HeadAtCycleStart,
    ) -> Option<IssueHold> {
        let oldest =
            self.slots.iter().flatten().min_by(|a, b| a.entry.rob_tag.age_cmp(b.entry.rob_tag))?;
        let Some(part) = oldest.ready_part() else { return Some(IssueHold::Operands) };
        let ordered =
            part == IssuePart::StoreData || Self::may_issue_now(oldest, store_buffer, rob, head);
        (!ordered).then_some(IssueHold::Ordering)
    }

    /// The entry's instruction with its operand values filled in.
    fn with_operands(iq: IssueQueueEntry) -> RenameIssueEntry {
        let mut entry = iq.entry;
        if entry.trap.is_none() {
            debug_assert!(
                !matches!(iq.src1.readiness, OperandReady::NotReady),
                "IQ select: src1 not ready for rob_tag={} pc={:#x}",
                entry.rob_tag,
                entry.inst.pc,
            );
            debug_assert!(
                !matches!(iq.src2.readiness, OperandReady::NotReady),
                "IQ select: src2 not ready for rob_tag={} pc={:#x}",
                entry.rob_tag,
                entry.inst.pc,
            );
            entry.inst.rv1 = Self::resolve_value(&iq.src1);
            entry.inst.rv2 = Self::resolve_value(&iq.src2);
            entry.inst.rv3 = Self::resolve_value(&iq.src3);
        }
        entry
    }

    /// Whether an entry whose operands are ready may issue this cycle under
    /// the memory-ordering and serialisation rules.
    fn may_issue_now(
        iq: &IssueQueueEntry,
        store_buffer: &StoreBuffer,
        rob: &Rob,
        head: HeadAtCycleStart,
    ) -> bool {
        let mem_ready = match &iq.mem_dep {
            MemDepState::None | MemDepState::Bypass | MemDepState::Resolved(_) => true,
            MemDepState::WaitAll => !store_buffer.has_unresolved_store_before(iq.entry.rob_tag),
            MemDepState::WaitFor(barrier) => !store_buffer.is_unresolved(*barrier),
        };
        if !mem_ready {
            return false;
        }
        // FENCE / CBO* have their own granular checks below. Every other
        // system instruction reads or writes architectural state, so it
        // executes only once it is the oldest instruction: nothing older can
        // still change that state or squash it.
        let ctrl = &iq.entry.inst.ctrl;
        if ctrl.system_op != SystemOp::None
            && ctrl.system_op != SystemOp::Fence
            && !ctrl.system_op.is_cbo()
            && !head.is(iq.entry.rob_tag)
        {
            return false;
        }
        // An AMO or store-conditional is non-speculative (gem5's
        // IsNonSpeculative): it executes only as the oldest.
        if ctrl.performs_at_rob_head() && !head.is(iq.entry.rob_tag) {
            return false;
        }
        {
            use crate::exec::compute::vector::mem::{is_vec_load, is_vec_store};
            if (is_vec_load(ctrl.vec_op) || is_vec_store(ctrl.vec_op))
                && (store_buffer.has_unresolved_store_before(iq.entry.rob_tag)
                    || store_buffer.has_committed_stores())
            {
                return false;
            }
        }
        if ctrl.system_op == SystemOp::Fence {
            let pred_bits = ((iq.entry.inst.bits >> 24) & 0xF) as u8;
            let pred_r = pred_bits & 0b0010 != 0;
            let pred_w = pred_bits & 0b0001 != 0;
            if !rob.fence_pred_satisfied(iq.entry.rob_tag, pred_r, pred_w) {
                return false;
            }
        }
        let reads = ctrl.reads_memory();
        let writes = ctrl.writes_memory();
        !((reads || writes) && rob.has_fence_blocking(iq.entry.rob_tag, reads, writes))
    }

    /// The operand value at select time; `NotReady` only for faulted
    /// instructions, which take no operands.
    #[inline]
    const fn resolve_value(src: &OperandState) -> u64 {
        match src.readiness {
            OperandReady::Ready(v) => v,
            OperandReady::NotReady => 0,
        }
    }

    /// Number of free slots available for dispatch.
    pub const fn available_slots(&self) -> usize {
        self.capacity - self.count
    }

    /// Flush all entries.
    pub fn flush(&mut self) {
        self.slots.fill(None);
        self.count = 0;
    }

    /// Flush entries newer than `keep_tag`.
    pub fn flush_after(&mut self, keep_tag: RobTag) {
        for slot in &mut self.slots {
            if let Some(iq) = slot
                && iq.entry.rob_tag.is_newer_than(keep_tag)
            {
                *slot = None;
                self.count -= 1;
            }
        }
    }

    /// Wake entries whose memory dependency barrier has resolved.
    ///
    /// Called when [`MemDepUnit::store_resolved`](crate::uarch::mdp::MemDepUnit)
    /// returns woken tags. Transitions `WaitFor` → `Resolved` so `select()` can issue them.
    pub fn wakeup_mem_dep(&mut self, resolved_tags: &[RobTag]) {
        for slot in self.slots.iter_mut().flatten() {
            if let MemDepState::WaitFor(barrier) = &slot.mem_dep
                && resolved_tags.contains(barrier)
            {
                slot.mem_dep = MemDepState::Resolved(*barrier);
            }
        }
    }

    /// Return a snapshot of all entries in the queue (sorted by `rob_tag`, oldest first).
    pub fn queue_snapshot(&self) -> Vec<RenameIssueEntry> {
        let mut entries: Vec<&IssueQueueEntry> =
            self.slots.iter().filter_map(|s| s.as_ref()).collect();
        entries.sort_by(|a, b| a.entry.rob_tag.age_cmp(b.entry.rob_tag));
        entries.into_iter().map(|iq| iq.entry.clone()).collect()
    }

    /// Whether the queue is empty.
    #[cfg(test)]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[cfg(test)]
    /// Current number of entries.
    pub const fn len(&self) -> usize {
        self.count
    }
}

/// Resolve an operand via the PRF (O3 path).
fn resolve_operand_prf(
    reg: RegIdx,
    is_fp: bool,
    phys: PhysReg,
    prf: &PhysRegFile,
    _state: &StageCtx<'_>,
) -> OperandState {
    if !is_fp && reg.is_zero() {
        return OperandState::ready(PhysReg(0), 0);
    }

    if prf.is_ready(phys) {
        OperandState::ready(phys, prf.read(phys))
    } else {
        if phys.0 == 0 {
            return OperandState::ready(PhysReg(0), 0);
        }
        OperandState::not_ready(phys)
    }
}

/// Resolve an operand's initial state at dispatch time (legacy scoreboard path).
fn resolve_operand_legacy(
    reg: RegIdx,
    is_fp: bool,
    tag: Option<RobTag>,
    rob: &Rob,
    state: &StageCtx<'_>,
) -> OperandState {
    if !is_fp && reg.is_zero() {
        return OperandState::ready(PhysReg(0), 0);
    }

    tag.map_or_else(
        || {
            let value =
                if is_fp { state.hart().regs.read_f(reg) } else { state.hart().regs.read(reg) };
            OperandState::ready(PhysReg(0), value)
        },
        |t| match rob.find_entry(t) {
            Some(entry) if entry.state == RobState::Completed => {
                OperandState::ready(PhysReg(0), entry.result.unwrap_or(0))
            }
            Some(_) => OperandState::not_ready(PhysReg(0)),
            None => {
                // ROB entry already committed — read from register file.
                let value =
                    if is_fp { state.hart().regs.read_f(reg) } else { state.hart().regs.read(reg) };
                OperandState::ready(PhysReg(0), value)
            }
        },
    )
}

#[cfg(test)]
mod tests;
