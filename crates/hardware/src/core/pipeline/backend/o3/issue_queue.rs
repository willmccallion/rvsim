//! CAM-style Issue Queue for the O3 backend.
//!
//! Instructions dispatched from rename sit in the issue queue until all source
//! operands are ready. The wakeup/select logic allows out-of-order issue:
//! - **Wakeup (PRF path)**: when an instruction completes, its `PhysReg` is broadcast
//!   to all waiting entries, marking matching source operands as ready.
//! - **Wakeup (legacy path)**: when an instruction completes, its ROB tag is broadcast.
//! - **Select**: each cycle, the oldest entries with all operands ready are
//!   selected for execution (up to `width`).

use crate::common::RegIdx;
use crate::core::exec::signals::{SystemOp, VectorOp};
use crate::core::pipeline::backend::o3::fu_pool::{FU_TYPE_COUNT, FreeUnit, FuPool, FuType};
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::prf::{PhysReg, PhysRegFile};
use crate::core::pipeline::rob::{Rob, RobState, RobTag};
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::core::pipeline::vec_prf::VecPhysRegFile;
use crate::core::units::mdp::MemDepState;
use crate::core::units::vpu::types::VecPhysReg;
use crate::sim::StageCtx;

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
    /// ROB tag of the producer (legacy path; None when using PRF).
    pub tag: Option<RobTag>,
    /// Readiness and value of this operand.
    pub readiness: OperandReady,
}

impl OperandState {
    /// Convenience: create a ready operand with a known value.
    const fn ready(phys: PhysReg, tag: Option<RobTag>, value: u64) -> Self {
        Self { phys, tag, readiness: OperandReady::Ready(value) }
    }

    /// Convenience: create a not-ready operand.
    const fn not_ready(phys: PhysReg, tag: Option<RobTag>) -> Self {
        Self { phys, tag, readiness: OperandReady::NotReady }
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
}

/// The instructions [`IssueQueue::select`] chose, and how many ready ones
/// it passed over because every unit of their class was busy.
#[derive(Debug, Default)]
pub struct Selection {
    /// Chosen instructions, oldest first.
    pub entries: Vec<SelectedEntry>,
    /// Ready instructions left waiting for a functional unit.
    pub unit_stalls: usize,
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
        if self.count >= self.capacity {
            return false;
        }

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
                OperandState::ready(PhysReg(0), None, 0)
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
                OperandState::ready(PhysReg(0), None, 0)
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
        };

        for slot in &mut self.slots {
            if slot.is_none() {
                *slot = Some(iq_entry);
                self.count += 1;
                return true;
            }
        }

        unreachable!("count < capacity but no free slot found");
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

    /// Broadcast a completed result via ROB tag (legacy wakeup path).
    pub fn wakeup(&mut self, tag: RobTag, value: u64) {
        for iq in self.slots.iter_mut().flatten() {
            if iq.src1.tag == Some(tag) && !iq.src1.readiness.is_ready() {
                iq.src1.readiness = OperandReady::Ready(value);
            }
            if iq.src2.tag == Some(tag) && !iq.src2.readiness.is_ready() {
                iq.src2.readiness = OperandReady::Ready(value);
            }
            if iq.src3.tag == Some(tag) && !iq.src3.readiness.is_ready() {
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

    /// Selects up to `budget.width` ready entries, oldest first (lowest
    /// `rob_tag.0`), as gem5's instruction scheduler does.
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
        let mut ready_indices: Vec<usize> = Vec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(iq) = slot {
                // Faulted instructions don't need operands — always ready.
                let all_ready = iq.entry.trap.is_some()
                    || (iq.src1.readiness.is_ready()
                        && iq.src2.readiness.is_ready()
                        && iq.src3.readiness.is_ready()
                        && iq.vec_src1.ready
                        && iq.vec_src2.ready
                        && iq.vec_src3.ready
                        && iq.mask_ready);
                if all_ready {
                    let mem_ready = match &iq.mem_dep {
                        MemDepState::None | MemDepState::Bypass | MemDepState::Resolved(_) => true,
                        MemDepState::WaitAll => {
                            !store_buffer.has_unresolved_store_before(iq.entry.rob_tag)
                        }
                        MemDepState::WaitFor(barrier) => !store_buffer.is_unresolved(*barrier),
                    };
                    if !mem_ready {
                        continue;
                    }
                    // FENCE / CBO* have their own granular checks below. Every
                    // other system instruction reads or writes architectural
                    // state, so it executes only once it is the oldest
                    // instruction: nothing older can still change that state
                    // or squash it.
                    if iq.entry.inst.ctrl.system_op != SystemOp::None
                        && iq.entry.inst.ctrl.system_op != SystemOp::Fence
                        && !iq.entry.inst.ctrl.system_op.is_cbo()
                        && !rob.is_head(iq.entry.rob_tag)
                    {
                        continue;
                    }
                    // An AMO or store-conditional is non-speculative (gem5's
                    // IsNonSpeculative): it executes only as the oldest.
                    if iq.entry.inst.ctrl.performs_at_rob_head() && !rob.is_head(iq.entry.rob_tag) {
                        continue;
                    }
                    {
                        use crate::core::units::vpu::mem::{is_vec_load, is_vec_store};
                        let vop = iq.entry.inst.ctrl.vec_op;
                        if (is_vec_load(vop) || is_vec_store(vop))
                            && (store_buffer.has_unresolved_store_before(iq.entry.rob_tag)
                                || store_buffer.has_committed_stores())
                        {
                            continue;
                        }
                    }
                    if iq.entry.inst.ctrl.system_op == SystemOp::Fence {
                        let pred_bits = ((iq.entry.inst.bits >> 24) & 0xF) as u8;
                        let pred_r = pred_bits & 0b0010 != 0;
                        let pred_w = pred_bits & 0b0001 != 0;
                        if !rob.fence_pred_satisfied(iq.entry.rob_tag, pred_r, pred_w) {
                            continue;
                        }
                    }
                    let reads = iq.entry.inst.ctrl.reads_memory();
                    let writes = iq.entry.inst.ctrl.writes_memory();
                    if (reads || writes) && rob.has_fence_blocking(iq.entry.rob_tag, reads, writes)
                    {
                        continue;
                    }
                    ready_indices.push(i);
                }
            }
        }

        ready_indices.sort_by_key(|&i| self.slots[i].as_ref().map_or(0, |s| s.entry.rob_tag.0));

        let mut selection = Selection::default();
        let mut loads_issued = 0usize;
        let mut stores_issued = 0usize;
        let mut units_taken = [0usize; FU_TYPE_COUNT];
        for &idx in &ready_indices {
            if selection.entries.len() >= budget.width {
                break;
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

            let Some(iq) = self.slots[idx].take() else { continue };
            self.count -= 1;
            if is_load {
                loads_issued += 1;
            }
            if is_store {
                stores_issued += 1;
            }

            let mut entry = iq.entry;
            if entry.trap.is_none() {
                debug_assert!(
                    !matches!(iq.src1.readiness, OperandReady::NotReady),
                    "IQ select: src1 not ready for rob_tag={} pc={:#x}",
                    entry.rob_tag.0,
                    entry.inst.pc,
                );
                debug_assert!(
                    !matches!(iq.src2.readiness, OperandReady::NotReady),
                    "IQ select: src2 not ready for rob_tag={} pc={:#x}",
                    entry.rob_tag.0,
                    entry.inst.pc,
                );
                entry.inst.rv1 = Self::resolve_value(&iq.src1);
                entry.inst.rv2 = Self::resolve_value(&iq.src2);
                entry.inst.rv3 = Self::resolve_value(&iq.src3);
            }
            selection.entries.push(SelectedEntry { entry, fu_type, unit });
        }

        selection
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
        for slot in &mut self.slots {
            *slot = None;
        }
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
    /// Called when [`MemDepUnit::store_resolved`](crate::core::units::mdp::MemDepUnit)
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
        entries.sort_by_key(|iq| iq.entry.rob_tag.0);
        entries.into_iter().map(|iq| iq.entry.clone()).collect()
    }

    /// Whether the queue is empty.
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

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
        return OperandState::ready(PhysReg(0), None, 0);
    }

    if prf.is_ready(phys) {
        OperandState::ready(phys, None, prf.read(phys))
    } else {
        if phys.0 == 0 {
            return OperandState::ready(PhysReg(0), None, 0);
        }
        OperandState::not_ready(phys, None)
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
        return OperandState::ready(PhysReg(0), None, 0);
    }

    tag.map_or_else(
        || {
            let value =
                if is_fp { state.hart().regs.read_f(reg) } else { state.hart().regs.read(reg) };
            OperandState::ready(PhysReg(0), None, value)
        },
        |t| match rob.find_entry(t) {
            Some(entry) if entry.state == RobState::Completed => {
                OperandState::ready(PhysReg(0), Some(t), entry.result.unwrap_or(0))
            }
            Some(_) => OperandState::not_ready(PhysReg(0), Some(t)),
            None => {
                // ROB entry already committed — read from register file.
                let value =
                    if is_fp { state.hart().regs.read_f(reg) } else { state.hart().regs.read(reg) };
                OperandState::ready(PhysReg(0), None, value)
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{InstSize, RegIdx};
    use crate::core::exec::inst::Inst;
    use crate::core::exec::signals::ControlSignals;
    use crate::core::pipeline::latches::RenameIssueEntry;
    use crate::core::pipeline::prf::PhysReg;
    use crate::core::pipeline::rob::RobTag;

    fn make_entry(rob_tag: u32) -> RenameIssueEntry {
        RenameIssueEntry {
            // NOP
            inst: Inst {
                pc: 0x1000 + (rob_tag as u64) * 4,
                bits: 0x13,
                size: InstSize::Standard,
                rs1: RegIdx::new(0),
                rs2: RegIdx::new(0),
                rs3: RegIdx::new(0),
                rd: RegIdx::new(1),
                imm: 0,
                rv1: 0,
                rv2: 0,
                rv3: 0,
                ctrl: ControlSignals::default(),
            },
            rob_tag: RobTag(rob_tag),
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: PhysReg(0),
            rs2_phys: PhysReg(0),
            rs3_phys: PhysReg(0),
            rd_phys: PhysReg(0),
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            seq: crate::common::InstSeq::default(),
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
            vec_vtype: 0,
            vec_vl: 0,
            vec_vstart: 0,
            vec_vxrm: 0,
            vec_frm: 0,
        }
    }

    /// Selects at cycle 0 with more address units than any test issues.
    fn select(
        iq: &mut IssueQueue,
        width: usize,
        store_buffer: &StoreBuffer,
        rob: &Rob,
        load_ports: usize,
        store_ports: usize,
    ) -> Vec<SelectedEntry> {
        use crate::core::pipeline::backend::o3::fu_pool::FuConfig;
        let units = FuPool::new(&FuConfig { num_mem: 8, ..FuConfig::default() });
        let budget = IssueBudget {
            width,
            load_ports,
            store_ports,
            units: &units,
            now: 0,
            memory_blocked: false,
        };
        iq.select(&budget, store_buffer, rob).entries
    }

    /// A ready entry for `rob_tag` executing `ctrl`.
    fn ready_entry(rob_tag: u32, ctrl: ControlSignals) -> IssueQueueEntry {
        let base = make_entry(rob_tag);
        IssueQueueEntry {
            entry: RenameIssueEntry { inst: Inst { ctrl, ..base.inst }, ..base },
            src1: ready_operand(0),
            src2: ready_operand(0),
            src3: ready_operand(0),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
        }
    }

    #[test]
    fn a_ready_op_whose_unit_is_busy_lets_a_younger_op_issue_in_its_place() {
        use crate::core::exec::signals::AluOp;
        use crate::core::pipeline::backend::o3::fu_pool::FuConfig;
        let mut units = FuPool::new(&FuConfig { num_int_div: 1, ..FuConfig::default() });
        let busy_divider = units.free_unit(FuType::IntDiv, 0).expect("a divider");
        let _ = units.acquire(busy_divider, 0);
        let mut iq = IssueQueue::new(8);
        iq.slots[0] =
            Some(ready_entry(1, ControlSignals { alu: AluOp::Div, ..Default::default() }));
        iq.slots[1] = Some(ready_entry(2, ControlSignals::default()));
        iq.count = 2;
        let budget = IssueBudget {
            width: 1,
            load_ports: 1,
            store_ports: 1,
            units: &units,
            now: 1,
            memory_blocked: false,
        };

        let selection = iq.select(&budget, &StoreBuffer::new(4), &Rob::new(8));

        let issued: Vec<u32> = selection.entries.iter().map(|e| e.entry.rob_tag.0).collect();
        assert_eq!((issued, selection.unit_stalls), (vec![2], 1));
    }

    #[test]
    fn a_blocked_memory_pipeline_holds_loads_but_not_alu_ops() {
        let units = FuPool::new(&crate::core::pipeline::backend::o3::fu_pool::FuConfig::default());
        let mut iq = IssueQueue::new(8);
        iq.slots[0] = Some(ready_entry(1, ControlSignals { mem_read: true, ..Default::default() }));
        iq.slots[1] = Some(ready_entry(2, ControlSignals::default()));
        iq.count = 2;
        let budget = IssueBudget {
            width: 1,
            load_ports: 1,
            store_ports: 1,
            units: &units,
            now: 0,
            memory_blocked: true,
        };

        let selection = iq.select(&budget, &StoreBuffer::new(4), &Rob::new(8));

        let issued: Vec<u32> = selection.entries.iter().map(|e| e.entry.rob_tag.0).collect();
        assert_eq!(issued, vec![2]);
    }

    fn ready_operand(value: u64) -> OperandState {
        OperandState::ready(PhysReg(0), None, value)
    }

    fn not_ready_operand_phys(phys: PhysReg) -> OperandState {
        OperandState::not_ready(phys, None)
    }

    fn not_ready_operand_tag(tag: RobTag) -> OperandState {
        OperandState::not_ready(PhysReg(0), Some(tag))
    }

    #[test]
    fn test_new_empty() {
        let iq = IssueQueue::new(16);
        assert!(iq.is_empty());
        assert_eq!(iq.available_slots(), 16);
    }

    #[test]
    fn test_dispatch_and_select_ready() {
        let mut iq = IssueQueue::new(16);

        // Manually insert a ready entry
        iq.slots[0] = Some(IssueQueueEntry {
            entry: make_entry(1),
            src1: ready_operand(42),
            src2: ready_operand(10),
            src3: ready_operand(0),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
        });
        iq.count = 1;

        let selected =
            select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].entry.rob_tag.0, 1);
        assert_eq!(selected[0].entry.inst.rv1, 42);
        assert_eq!(selected[0].entry.inst.rv2, 10);
        assert!(iq.is_empty());
    }

    #[test]
    fn test_wakeup_phys_chain() {
        let mut iq = IssueQueue::new(16);
        let p5 = PhysReg(5);

        // Entry depends on phys reg 5
        let entry = make_entry(10);
        iq.slots[0] = Some(IssueQueueEntry {
            entry,
            src1: not_ready_operand_phys(p5),
            src2: ready_operand(0),
            src3: ready_operand(0),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
        });
        iq.count = 1;

        // Not ready yet
        let selected =
            select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
        assert_eq!(selected.len(), 0);

        // Wakeup with phys reg 5
        iq.wakeup_phys(p5, 999);

        // Now should be selectable
        let selected =
            select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].entry.inst.rv1, 999);
    }

    #[test]
    fn test_wakeup_legacy_chain() {
        let mut iq = IssueQueue::new(16);

        // Entry depends on tag 5
        let entry = make_entry(10);
        iq.slots[0] = Some(IssueQueueEntry {
            entry,
            src1: not_ready_operand_tag(RobTag(5)),
            src2: ready_operand(0),
            src3: ready_operand(0),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
        });
        iq.count = 1;

        // Wakeup with tag 5
        iq.wakeup(RobTag(5), 999);

        let selected =
            select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].entry.inst.rv1, 999);
    }

    #[test]
    fn test_oldest_first_select() {
        let mut iq = IssueQueue::new(16);

        // Insert entries with tags 3, 1, 2 in random slot order
        for (slot, tag) in [(2, 3u32), (0, 1), (1, 2)] {
            iq.slots[slot] = Some(IssueQueueEntry {
                entry: make_entry(tag),
                src1: ready_operand(tag as u64),
                src2: ready_operand(0),
                src3: ready_operand(0),
                vec_src1: VecOperandState::default(),
                vec_src2: VecOperandState::default(),
                vec_src3: VecOperandState::default(),
                mem_dep: MemDepState::None,
                mask_phys: VecPhysReg::ZERO,
                mask_ready: true,
                needs_mask: false,
            });
        }
        iq.count = 3;

        // Select width=2 should get tags 1 and 2 (oldest first)
        let selected =
            select(&mut iq, 2, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].entry.rob_tag.0, 1);
        assert_eq!(selected[1].entry.rob_tag.0, 2);
        assert_eq!(iq.len(), 1);

        // Remaining is tag 3
        let selected =
            select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].entry.rob_tag.0, 3);
    }

    #[test]
    fn test_flush() {
        let mut iq = IssueQueue::new(16);
        iq.slots[0] = Some(IssueQueueEntry {
            entry: make_entry(1),
            src1: OperandState::default(),
            src2: OperandState::default(),
            src3: OperandState::default(),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
        });
        iq.slots[5] = Some(IssueQueueEntry {
            entry: make_entry(2),
            src1: OperandState::default(),
            src2: OperandState::default(),
            src3: OperandState::default(),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
        });
        iq.count = 2;

        iq.flush();
        assert!(iq.is_empty());
        assert_eq!(iq.available_slots(), 16);
    }

    #[test]
    fn test_flush_after() {
        let mut iq = IssueQueue::new(16);
        for (slot, tag) in [(0, 1u32), (1, 2), (2, 3), (3, 4)] {
            iq.slots[slot] = Some(IssueQueueEntry {
                entry: make_entry(tag),
                src1: OperandState::default(),
                src2: OperandState::default(),
                src3: OperandState::default(),
                vec_src1: VecOperandState::default(),
                vec_src2: VecOperandState::default(),
                vec_src3: VecOperandState::default(),
                mem_dep: MemDepState::None,
                mask_phys: VecPhysReg::ZERO,
                mask_ready: true,
                needs_mask: false,
            });
        }
        iq.count = 4;

        // Keep tags <= 2
        iq.flush_after(RobTag(2));
        assert_eq!(iq.len(), 2);

        let snap = iq.queue_snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].rob_tag.0, 1);
        assert_eq!(snap[1].rob_tag.0, 2);
    }

    #[test]
    fn test_queue_snapshot_sorted() {
        let mut iq = IssueQueue::new(16);
        // Insert in reverse order
        for (slot, tag) in [(0, 5u32), (1, 3), (2, 1)] {
            iq.slots[slot] = Some(IssueQueueEntry {
                entry: make_entry(tag),
                src1: OperandState::default(),
                src2: OperandState::default(),
                src3: OperandState::default(),
                vec_src1: VecOperandState::default(),
                vec_src2: VecOperandState::default(),
                vec_src3: VecOperandState::default(),
                mem_dep: MemDepState::None,
                mask_phys: VecPhysReg::ZERO,
                mask_ready: true,
                needs_mask: false,
            });
        }
        iq.count = 3;

        let snap = iq.queue_snapshot();
        assert_eq!(snap.len(), 3);
        assert_eq!(snap[0].rob_tag.0, 1);
        assert_eq!(snap[1].rob_tag.0, 3);
        assert_eq!(snap[2].rob_tag.0, 5);
    }

    #[test]
    fn a_vector_load_waits_behind_an_incomplete_acquire_atomic() {
        let mut rob = Rob::new(8);
        let acquire = ControlSignals {
            atomic_op: crate::core::exec::signals::AtomicOp::Swap,
            acquire: true,
            mem_read: true,
            mem_write: true,
            ..Default::default()
        };
        let vector_load = ControlSignals {
            vec_op: crate::core::exec::signals::VectorOp::VLoadUnit,
            ..Default::default()
        };
        let alloc = |rob: &mut Rob, ctrl| {
            rob.allocate(
                0,
                0,
                InstSize::Standard,
                RegIdx::new(0),
                false,
                ctrl,
                PhysReg(0),
                PhysReg(0),
                crate::common::InstSeq::default(),
            )
            .unwrap()
        };
        let amo_tag = alloc(&mut rob, acquire);
        let load_tag = alloc(&mut rob, vector_load);
        let mut iq = IssueQueue::new(4);
        let mut entry = make_entry(load_tag.0);
        entry.inst.ctrl = vector_load;
        iq.slots[0] = Some(IssueQueueEntry {
            entry,
            src1: ready_operand(0),
            src2: ready_operand(0),
            src3: ready_operand(0),
            vec_src1: VecOperandState::default(),
            vec_src2: VecOperandState::default(),
            vec_src3: VecOperandState::default(),
            mem_dep: MemDepState::None,
            mask_phys: VecPhysReg::ZERO,
            mask_ready: true,
            needs_mask: false,
        });
        iq.count = 1;

        assert!(select(&mut iq, 4, &StoreBuffer::new(4), &rob, 2, 1).is_empty());
        rob.complete(amo_tag, 0);
        assert_eq!(select(&mut iq, 4, &StoreBuffer::new(4), &rob, 2, 1).len(), 1);
    }

    #[test]
    fn test_port_limits() {
        let mut iq = IssueQueue::new(16);

        // Insert 3 loads (tags 1, 2, 3) and 2 stores (tags 4, 5), all ready
        for (slot, tag, is_load, is_store) in [
            (0, 1u32, true, false),
            (1, 2, true, false),
            (2, 3, true, false),
            (3, 4, false, true),
            (4, 5, false, true),
        ] {
            let mut entry = make_entry(tag);
            entry.inst.ctrl.mem_read = is_load;
            entry.inst.ctrl.mem_write = is_store;
            iq.slots[slot] = Some(IssueQueueEntry {
                entry,
                src1: ready_operand(0),
                src2: ready_operand(0),
                src3: ready_operand(0),
                vec_src1: VecOperandState::default(),
                vec_src2: VecOperandState::default(),
                vec_src3: VecOperandState::default(),
                mem_dep: MemDepState::None,
                mask_phys: VecPhysReg::ZERO,
                mask_ready: true,
                needs_mask: false,
            });
        }
        iq.count = 5;

        // With load_ports=2, store_ports=1, width=4: should get 2 loads + 1 store = 3
        let selected = select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), 2, 1);
        assert_eq!(selected.len(), 3);
        // Oldest first: tags 1 (load), 2 (load), 4 (store)
        assert_eq!(selected[0].entry.rob_tag.0, 1);
        assert!(selected[0].entry.inst.ctrl.mem_read);
        assert_eq!(selected[1].entry.rob_tag.0, 2);
        assert!(selected[1].entry.inst.ctrl.mem_read);
        assert_eq!(selected[2].entry.rob_tag.0, 4);
        assert!(selected[2].entry.inst.ctrl.mem_write);

        // Remaining: tag 3 (load), tag 5 (store)
        assert_eq!(iq.len(), 2);

        // Next cycle: should get remaining load + store
        let selected = select(&mut iq, 4, &StoreBuffer::new(16), &Rob::new(64), 2, 1);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].entry.rob_tag.0, 3);
        assert_eq!(selected[1].entry.rob_tag.0, 5);
        assert!(iq.is_empty());
    }
}
