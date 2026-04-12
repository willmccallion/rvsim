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
use crate::core::Cpu;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::prf::{PhysReg, PhysRegFile};
use crate::core::pipeline::rob::{Rob, RobState, RobTag};
use crate::core::pipeline::signals::SystemOp;
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::core::pipeline::vec_prf::VecPhysRegFile;
use crate::core::units::mdp::MemDepState;
use crate::core::units::vpu::types::VecPhysReg;

/// Readiness state of a single source operand.
///
/// Encodes the three mutually-exclusive states an operand can be in,
/// making it impossible to read a value that isn't ready.
#[derive(Clone, Copy, Debug, Default)]
pub enum OperandReady {
    /// Value not available yet — waiting on producer.
    #[default]
    NotReady,
    /// Speculatively woken by a load (assuming L1D hit).
    /// The PRF has NOT been written yet — the real value must be read
    /// from the PRF at select time after validation.
    Speculative,
    /// Ready with a real value.
    Ready(u64),
}

impl OperandReady {
    /// Returns `true` if the operand is ready (either `Ready` or `Speculative`).
    #[inline]
    pub const fn is_ready(self) -> bool {
        !matches!(self, Self::NotReady)
    }

    /// Returns `true` if speculative (load-hit speculation, not yet confirmed).
    #[inline]
    pub const fn is_speculative(self) -> bool {
        matches!(self, Self::Speculative)
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
        // Only re-check if this group contains the woken register
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
    /// The memory dependency state from dispatch (must be preserved on re-dispatch).
    pub mem_dep: MemDepState,
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
        cpu: &Cpu,
        prf: Option<&PhysRegFile>,
        vec_prf: Option<&VecPhysRegFile>,
        mem_dep: MemDepState,
    ) -> bool {
        if self.count >= self.capacity {
            return false;
        }

        let (src1, src2, src3) = if let Some(prf) = prf {
            // PRF path: check ready bits in the physical register file
            let s1 = resolve_operand_prf(entry.rs1, entry.ctrl.rs1_fp, entry.rs1_phys, prf, cpu);
            let s2 = resolve_operand_prf(entry.rs2, entry.ctrl.rs2_fp, entry.rs2_phys, prf, cpu);
            let s3 = if entry.ctrl.rs3_fp {
                resolve_operand_prf(entry.rs3, true, entry.rs3_phys, prf, cpu)
            } else {
                OperandState::ready(PhysReg(0), None, 0)
            };
            (s1, s2, s3)
        } else {
            // Legacy scoreboard path: check ROB completion
            let s1 = resolve_operand_legacy(entry.rs1, entry.ctrl.rs1_fp, entry.rs1_tag, rob, cpu);
            let s2 = resolve_operand_legacy(entry.rs2, entry.ctrl.rs2_fp, entry.rs2_tag, rob, cpu);
            let s3 = if entry.ctrl.rs3_fp {
                resolve_operand_legacy(entry.rs3, true, entry.rs3_tag, rob, cpu)
            } else {
                OperandState::ready(PhysReg(0), None, 0)
            };
            (s1, s2, s3)
        };

        // Initialize vector operand states (default ready if no vec sources)
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

        // Check initial readiness against vec PRF (sources may already be ready)
        if let Some(vprf) = vec_prf {
            vec_src1.check_ready(vprf);
            vec_src2.check_ready(vprf);
            vec_src3.check_ready(vprf);
        }

        // Track v0 mask register dependency for masked vector ops (vm=0).
        let needs_mask = !entry.ctrl.vm
            && entry.ctrl.vec_op != crate::core::pipeline::signals::VectorOp::None
            && !matches!(
                entry.ctrl.vec_op,
                crate::core::pipeline::signals::VectorOp::Vsetvli
                    | crate::core::pipeline::signals::VectorOp::Vsetivli
                    | crate::core::pipeline::signals::VectorOp::Vsetvl
            );
        let mask_phys = if needs_mask {
            // v0 physical register from the rename map (captured in entry)
            // v0 is at VRegIdx(0) — its physical mapping is vs1_phys[0] only
            // if vs1 == v0. We need the rename map's mapping for v0, which
            // is not directly in the entry. Use the vec_prf identity for now:
            // the rename stage should populate this. For now, read from the
            // first slot of the rename map via a helper on the entry.
            //
            // The correct mapping is stored by rename in the entry's vs-phys
            // arrays only if vs1/vs2/vs3 happens to be v0. We need a dedicated
            // field. For now, we'll add it to RenameIssueEntry.
            //
            // TEMPORARY: use VecPhysReg::ZERO (always ready) until
            // RenameIssueEntry is extended with mask_phys field.
            entry.mask_phys
        } else {
            VecPhysReg::ZERO
        };
        let mask_ready = if needs_mask {
            if let Some(vprf) = vec_prf {
                vprf.is_ready(mask_phys)
            } else {
                true
            }
        } else {
            true
        };

        let iq_entry = IssueQueueEntry {
            entry, src1, src2, src3, vec_src1, vec_src2, vec_src3, mem_dep,
            mask_phys, mask_ready, needs_mask,
        };

        // Find first free slot
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

    /// Speculatively mark operands waiting on `p` as ready.
    ///
    /// Used for load speculation: when a load issues, we optimistically wake
    /// dependents so they can be selected next cycle (assuming L1D hit).
    /// The PRF is NOT written — `select()` validates against PRF before issuing.
    /// If the load hits, normal writeback will write the PRF and re-wakeup with
    /// the real value. If it misses, `cancel_wakeup_phys()` reverts this.
    pub fn speculative_wakeup_phys(&mut self, p: PhysReg) {
        if p.0 == 0 {
            return;
        }
        for iq in self.slots.iter_mut().flatten() {
            if iq.src1.phys == p && !iq.src1.readiness.is_ready() {
                iq.src1.readiness = OperandReady::Speculative;
            }
            if iq.src2.phys == p && !iq.src2.readiness.is_ready() {
                iq.src2.readiness = OperandReady::Speculative;
            }
            if iq.src3.phys == p && !iq.src3.readiness.is_ready() {
                iq.src3.readiness = OperandReady::Speculative;
            }
        }
    }

    /// Cancel a speculative wakeup: revert operands waiting on `p` to not-ready.
    ///
    /// Called when a speculatively-woken load turns out to be an L1D miss.
    /// Only reverts operands whose PRF entry is still not-ready (the speculative
    /// ones). Operands that have since been written by a real wakeup are unaffected.
    pub fn cancel_wakeup_phys(&mut self, p: PhysReg, prf: &PhysRegFile) {
        if p.0 == 0 {
            return;
        }
        // Only cancel if the PRF says this register is still not ready
        // (i.e., no real wakeup has arrived yet).
        if prf.is_ready(p) {
            return;
        }
        for iq in self.slots.iter_mut().flatten() {
            if iq.src1.phys == p && iq.src1.readiness.is_speculative() {
                iq.src1.readiness = OperandReady::NotReady;
            }
            if iq.src2.phys == p && iq.src2.readiness.is_speculative() {
                iq.src2.readiness = OperandReady::NotReady;
            }
            if iq.src3.phys == p && iq.src3.readiness.is_speculative() {
                iq.src3.readiness = OperandReady::NotReady;
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
            // Mask register v0 wakeup
            if iq.needs_mask && !iq.mask_ready && iq.mask_phys == p {
                iq.mask_ready = vec_prf.is_ready(p);
            }
        }
    }

    /// Select up to `width` ready entries, oldest first (lowest `rob_tag.0`).
    ///
    /// Selected entries have their `rv1/rv2/rv3` fields populated from the
    /// resolved operand values. The slots are freed.
    ///
    /// When `prf` is provided, operands that were speculatively marked ready
    /// are validated: if the IQ says ready but the PRF says not-ready, the
    /// operand was speculatively woken by a load that hasn't completed yet.
    /// Such entries are skipped (not issued).
    ///
    /// Memory dependencies are checked via the cached [`MemDepState`] set at
    /// dispatch time, rather than re-querying the predictor every cycle.
    ///
    /// System/CSR instructions are serializing: they must not issue until all
    /// older ROB entries have completed.
    ///
    /// Memory port limits: at most `load_ports` loads and `store_ports` stores
    /// are issued per cycle, modeling finite LSU bandwidth.
    pub fn select(
        &mut self,
        width: usize,
        store_buffer: &StoreBuffer,
        rob: &Rob,
        load_ports: usize,
        store_ports: usize,
        prf: Option<&PhysRegFile>,
    ) -> Vec<SelectedEntry> {
        // Collect indices of all ready entries
        let mut ready_indices: Vec<usize> = Vec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(iq) = slot {
                // Faulted instructions don't need operands — always ready
                let all_ready = iq.entry.trap.is_some()
                    || (iq.src1.readiness.is_ready()
                        && iq.src2.readiness.is_ready()
                        && iq.src3.readiness.is_ready()
                        && iq.vec_src1.ready
                        && iq.vec_src2.ready
                        && iq.vec_src3.ready
                        && iq.mask_ready);
                // PRF validation: if an operand was speculatively woken (IQ says
                // ready) but the PRF still says not-ready, the speculative wakeup
                // hasn't been confirmed yet — treat as not ready.
                let prf_valid = prf.is_none_or(|prf| {
                    Self::prf_validated(&iq.src1, prf)
                        && Self::prf_validated(&iq.src2, prf)
                        && Self::prf_validated(&iq.src3, prf)
                });
                let all_ready = all_ready && prf_valid;
                if all_ready {
                    // Memory dependency check (cached at dispatch time).
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
                    // System/CSR instructions (excluding FENCE) are serializing:
                    // wait for all older instructions to complete before issuing.
                    // FENCE is excluded here because it has its own granular check
                    // below that only waits for operations matching its pred bits,
                    // rather than draining the entire pipeline.
                    if iq.entry.ctrl.system_op != SystemOp::None
                        && iq.entry.ctrl.system_op != SystemOp::Fence
                        && !rob.all_before_completed(iq.entry.rob_tag)
                    {
                        continue;
                    }
                    // Vector memory ops: store ordering. SB entries are allocated
                    // at execute time and commit-drained, so we only need to wait
                    // for older unresolved stores + committed store drain.
                    {
                        use crate::core::units::vpu::mem::{is_vec_load, is_vec_store};
                        let vop = iq.entry.ctrl.vec_op;
                        if (is_vec_load(vop) || is_vec_store(vop))
                            && (store_buffer.has_unresolved_store_before(iq.entry.rob_tag)
                                || store_buffer.has_committed_stores())
                        {
                            continue;
                        }
                    }
                    // FENCE: wait for older operations matching pred bits to complete.
                    if iq.entry.ctrl.system_op == SystemOp::Fence {
                        let pred_bits = ((iq.entry.inst >> 24) & 0xF) as u8;
                        let pred_r = pred_bits & 0b0010 != 0;
                        let pred_w = pred_bits & 0b0001 != 0;
                        if !rob.fence_pred_satisfied(iq.entry.rob_tag, pred_r, pred_w) {
                            continue;
                        }
                    }
                    // Loads/stores: blocked by older in-flight FENCE with matching succ bits.
                    if (iq.entry.ctrl.mem_read || iq.entry.ctrl.mem_write)
                        && rob.has_fence_blocking(
                            iq.entry.rob_tag,
                            iq.entry.ctrl.mem_read,
                            iq.entry.ctrl.mem_write,
                        )
                    {
                        continue;
                    }
                    ready_indices.push(i);
                }
            }
        }

        // Sort by rob_tag (oldest first = lowest tag value)
        ready_indices.sort_by_key(|&i| self.slots[i].as_ref().map_or(0, |s| s.entry.rob_tag.0));

        // Take up to `width`, respecting per-type port limits
        let mut result: Vec<SelectedEntry> = Vec::with_capacity(width);
        let mut loads_issued = 0usize;
        let mut stores_issued = 0usize;
        for &idx in &ready_indices {
            if result.len() >= width {
                break;
            }
            let Some(slot) = self.slots[idx].as_ref() else { continue };
            let ctrl = &slot.entry.ctrl;
            let is_load = ctrl.mem_read;
            let is_store = ctrl.mem_write;
            if is_load && loads_issued >= load_ports {
                continue; // port limit reached; entry stays in IQ
            }
            if is_store && stores_issued >= store_ports {
                continue;
            }

            let Some(iq) = self.slots[idx].take() else { continue };
            self.count -= 1;
            if is_load {
                loads_issued += 1;
            }
            if is_store {
                stores_issued += 1;
            }

            let mem_dep = iq.mem_dep;
            let mut entry = iq.entry;
            // Populate operand values from resolved state.
            if entry.trap.is_none() {
                // Assert all operands are genuinely ready at select time.
                // A NotReady operand would silently resolve to 0, which could
                // cause loads/stores to wrong addresses or corrupt register values.
                debug_assert!(
                    !matches!(iq.src1.readiness, OperandReady::NotReady),
                    "IQ select: src1 not ready for rob_tag={} pc={:#x}",
                    entry.rob_tag.0,
                    entry.pc,
                );
                debug_assert!(
                    !matches!(iq.src2.readiness, OperandReady::NotReady),
                    "IQ select: src2 not ready for rob_tag={} pc={:#x}",
                    entry.rob_tag.0,
                    entry.pc,
                );
                entry.rv1 = Self::resolve_value(&iq.src1, prf);
                entry.rv2 = Self::resolve_value(&iq.src2, prf);
                entry.rv3 = Self::resolve_value(&iq.src3, prf);
            }
            result.push(SelectedEntry { entry, mem_dep });
        }

        result
    }

    /// Check whether a speculatively-ready operand is validated by the PRF.
    ///
    /// Returns `true` if the operand is genuinely ready, or was never
    /// speculatively woken. Returns `false` if it was speculatively woken
    /// and the PRF still hasn't been written (load hasn't completed yet).
    #[inline]
    fn prf_validated(src: &OperandState, prf: &PhysRegFile) -> bool {
        match src.readiness {
            OperandReady::Speculative => prf.is_ready(src.phys),
            _ => true, // NotReady or Ready — no PRF validation needed
        }
    }

    /// Resolve the final operand value at select time.
    ///
    /// For `Ready(v)`, returns `v` directly.
    /// For `Speculative`, reads the real value from the PRF (which has been
    /// written by the real wakeup by the time we reach select).
    /// For `NotReady`, returns 0 (should only happen for faulted instructions).
    #[inline]
    fn resolve_value(src: &OperandState, prf: Option<&PhysRegFile>) -> u64 {
        match src.readiness {
            OperandReady::Ready(v) => v,
            OperandReady::Speculative => {
                if src.phys.0 == 0 {
                    return 0;
                }
                prf.map_or(0, |prf| prf.read(src.phys))
            }
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
    _cpu: &Cpu,
) -> OperandState {
    // x0 is hardwired zero
    if !is_fp && reg.is_zero() {
        return OperandState::ready(PhysReg(0), None, 0);
    }

    if prf.is_ready(phys) {
        OperandState::ready(phys, None, prf.read(phys))
    } else {
        // Not ready yet — will be woken up by wakeup_phys
        // For x0 (phys == PhysReg(0)), always ready
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
    cpu: &Cpu,
) -> OperandState {
    // x0 is hardwired zero
    if !is_fp && reg.is_zero() {
        return OperandState::ready(PhysReg(0), None, 0);
    }

    tag.map_or_else(
        || {
            // No in-flight producer — read from architectural register file
            let value = if is_fp { cpu.regs.read_f(reg) } else { cpu.regs.read(reg) };
            OperandState::ready(PhysReg(0), None, value)
        },
        |t| {
            // Check if ROB entry has completed
            match rob.find_entry(t) {
                Some(entry) if entry.state == RobState::Completed => {
                    OperandState::ready(PhysReg(0), Some(t), entry.result.unwrap_or(0))
                }
                Some(_) => OperandState::not_ready(PhysReg(0), Some(t)),
                None => {
                    // ROB entry already committed — read from register file
                    let value = if is_fp { cpu.regs.read_f(reg) } else { cpu.regs.read(reg) };
                    OperandState::ready(PhysReg(0), None, value)
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::InstSize;
    use crate::core::pipeline::latches::RenameIssueEntry;
    use crate::core::pipeline::prf::PhysReg;
    use crate::core::pipeline::rob::RobTag;
    use crate::core::pipeline::signals::ControlSignals;

    fn make_entry(rob_tag: u32) -> RenameIssueEntry {
        RenameIssueEntry {
            rob_tag: RobTag(rob_tag),
            pc: 0x1000 + (rob_tag as u64) * 4,
            inst: 0x13, // NOP
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
            imm: 0,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: PhysReg(0),
            rs2_phys: PhysReg(0),
            rs3_phys: PhysReg(0),
            rd_phys: PhysReg(0),
            ctrl: ControlSignals::default(),
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            ghr_snapshot: crate::core::units::bru::Ghr::default(),
            ras_snapshot: 0,
            vs1_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs2_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vs3_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vd_phys: [crate::core::units::vpu::types::VecPhysReg::ZERO; 8],
            vec_src1_count: 0,
            vec_src2_count: 0,
            vec_src3_count: 0,
            mask_phys: crate::core::units::vpu::types::VecPhysReg::ZERO,
        }
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
            iq.select(4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].entry.rob_tag.0, 1);
        assert_eq!(selected[0].entry.rv1, 42);
        assert_eq!(selected[0].entry.rv2, 10);
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
            iq.select(4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
        assert_eq!(selected.len(), 0);

        // Wakeup with phys reg 5
        iq.wakeup_phys(p5, 999);

        // Now should be selectable
        let selected =
            iq.select(4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].entry.rv1, 999);
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
            iq.select(4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].entry.rv1, 999);
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
            iq.select(2, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].entry.rob_tag.0, 1);
        assert_eq!(selected[1].entry.rob_tag.0, 2);
        assert_eq!(iq.len(), 1);

        // Remaining is tag 3
        let selected =
            iq.select(4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
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
            entry.ctrl.mem_read = is_load;
            entry.ctrl.mem_write = is_store;
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
        let selected = iq.select(4, &StoreBuffer::new(16), &Rob::new(64), 2, 1, None);
        assert_eq!(selected.len(), 3);
        // Oldest first: tags 1 (load), 2 (load), 4 (store)
        assert_eq!(selected[0].entry.rob_tag.0, 1);
        assert!(selected[0].entry.ctrl.mem_read);
        assert_eq!(selected[1].entry.rob_tag.0, 2);
        assert!(selected[1].entry.ctrl.mem_read);
        assert_eq!(selected[2].entry.rob_tag.0, 4);
        assert!(selected[2].entry.ctrl.mem_write);

        // Remaining: tag 3 (load), tag 5 (store)
        assert_eq!(iq.len(), 2);

        // Next cycle: should get remaining load + store
        let selected = iq.select(4, &StoreBuffer::new(16), &Rob::new(64), 2, 1, None);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].entry.rob_tag.0, 3);
        assert_eq!(selected[1].entry.rob_tag.0, 5);
        assert!(iq.is_empty());
    }
}
