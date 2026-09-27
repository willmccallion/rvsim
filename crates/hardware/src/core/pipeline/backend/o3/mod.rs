//! Out-of-Order (O3) backend: issue queue with wakeup/select, out-of-order execution.
//!
//! The O3 backend reuses shared pipeline stages (Memory1, Memory2, Writeback,
//! Commit) and shared hardware units (ALU, FPU, BRU), but has its own:
//! - **`IssueQueue`**: CAM-style with wakeup/select (vs FIFO for in-order)
//! - **`execute_one()`**: single-instruction execute (vs batch for in-order)

pub mod execute;
pub mod fu_pool;
pub mod issue_queue;
mod rename;
mod serialize;

use crate::config::Config;
use crate::core::pipeline::backend::shared::commit::{
    CommitEvent, CommitRegisters, CommitResources,
};
use crate::core::pipeline::backend::shared::vec_mem::{
    VecMemInflight, VecMemMicroOp, mem_width_from_eew_bytes, micro_ops_for, retire_element,
};
use crate::core::pipeline::backend::shared::{commit, memory1, memory2, writeback};
use crate::core::pipeline::checkpoint::CheckpointTable;
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::free_list::FreeList;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry, Mem2WbEntry, RenameIssueEntry};
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::prf::{PhysReg, PhysRegFile};
use crate::core::pipeline::rename_map::RenameMap;
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::signals::{AtomicOp, ControlFlow};
use crate::core::pipeline::squash::{PendingSquash, Redirect, SquashCause};
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::core::pipeline::vec_prf::VecPhysRegFile;
use crate::core::pipeline::vec_prf::VecPrfView;
use crate::core::pipeline::vec_store_buffer::VecStoreBuffer;
use crate::core::units::mdp::MemDepUnit;
use crate::core::units::vpu::chaining::VecPendingResult;
use crate::core::units::vpu::mem::{generate_element_addrs_vrf, is_vec_store};
use crate::core::units::vpu::types::{ElemIdx, NumLanes, VRegIdx, VecPhysReg, Vlen};
use crate::sim::CoreCtx;

use self::fu_pool::{FuPool, FuType};
use self::issue_queue::IssueQueue;

/// A result a functional unit is still producing. Its dependents wake, and
/// its ROB entry completes, when `complete_cycle` arrives.
#[derive(Debug)]
pub struct PendingResult {
    /// The execute-stage result entry.
    pub entry: ExMem1Entry,
    /// Cycle at which the result is ready (wakeup fires at this cycle).
    pub complete_cycle: u64,
    /// Functional unit type (for stats).
    pub fu_type: FuType,
}

/// Out-of-order execution engine.
#[derive(Debug)]
pub struct O3Engine {
    /// Reorder buffer.
    pub rob: Rob,
    /// Store buffer.
    pub store_buffer: StoreBuffer,
    /// Load queue for memory ordering violation detection.
    pub load_queue: LoadQueue,
    /// Physical register file (64-bit values + ready bits).
    pub prf: PhysRegFile,
    /// Free list of available physical register indices.
    pub free_list: FreeList<PhysReg>,
    /// Speculative rename map: arch reg → physical reg.
    pub rename_map: RenameMap,
    /// Committed rename map — restored on full trap flush.
    pub committed_rename_map: RenameMap,
    /// CAM-style issue queue with wakeup/select.
    pub issue_queue: IssueQueue,
    /// Functional unit pool for structural hazard modeling.
    pub fu_pool: FuPool,
    /// Results that have been computed but not yet written back.
    pub pending_results: Vec<PendingResult>,
    /// Pipeline width (max instructions issued/committed per cycle).
    pub width: usize,
    /// Instructions renamed and dispatched per cycle.
    rename_width: usize,
    /// Instructions issued per cycle.
    issue_width: usize,
    /// Instructions retired per cycle.
    commit_width: usize,
    /// Maximum loads issued per cycle.
    pub load_ports: usize,
    /// Maximum stores issued per cycle.
    pub store_ports: usize,
    /// Execute -> Memory1 latch.
    pub execute_mem1: Vec<ExMem1Entry>,
    /// Memory1 -> Memory2 latch.
    pub mem1_mem2: Vec<Mem1Mem2Entry>,
    /// Memory2 -> Writeback latch.
    pub mem2_wb: Vec<Mem2WbEntry>,
    /// Current simulation cycle (for FU latency tracking).
    pub cycle: u64,
    /// Memory dependence unit for load-store ordering speculation.
    pub mdp: MemDepUnit,
    /// Checkpoint table for O(1) branch misprediction recovery.
    pub checkpoints: CheckpointTable,
    /// Stall cycles remaining for in-progress squash recovery (blocks dispatch while > 0).
    pub squash_stall_remaining: u64,
    /// Whether rename holds the instruction after a serializing one.
    serialization: serialize::Serialization,
    /// Cycles from a result that redirects to the squash being taken.
    redirect_latency: u64,
    /// Vector physical register file (VLEN-bit storage per register + ready bits).
    pub vec_prf: VecPhysRegFile,
    /// Vector physical register free list.
    pub vec_free_list: FreeList<VecPhysReg>,
    /// Pending vector results tracking chaining wakeup and completion.
    pub vec_pending: Vec<VecPendingResult>,
    /// Number of vector execution lanes (derived from VLEN / 64, min 1).
    pub num_vec_lanes: NumLanes,
    /// Pending vector memory micro-ops waiting to enter the memory pipeline.
    pub vec_mem_pending: std::collections::VecDeque<VecMemMicroOp>,
    /// Tracks in-flight vector memory instructions and their remaining element count.
    pub vec_mem_inflight: Vec<VecMemInflight>,
    /// Dedicated store buffer for in-flight vector stores. See
    /// `core::pipeline::vec_store_buffer` for forwarding/drain semantics.
    pub vec_store_buffer: VecStoreBuffer,
    /// In-flight memory bookkeeping: mailbox + outstanding tables + routing IDs.
    pub common: crate::core::pipeline::engine::BackendCommon,
}

impl O3Engine {
    /// Creates a new O3 engine from config and routing IDs.
    pub fn new(
        config: &Config,
        pipeline_id: crate::sim::components::PipelineId,
        l1_i_id: crate::sim::components::CacheId,
        l1_d_id: crate::sim::components::CacheId,
    ) -> Self {
        let rob_size = config.pipeline.rob_size;
        let prf_gpr_size = config.pipeline.prf_gpr_size;
        let prf_fpr_size = config.pipeline.prf_fpr_size;
        let prf_total = prf_gpr_size + prf_fpr_size;
        // Slots 0..32 = GPR, 32..64 = FPR; free list starts at slot 64.
        let num_arch = 64;

        let mut prf = PhysRegFile::new(prf_total);
        prf.mark_arch_ready(num_arch);

        let fu_pool = FuPool::new(&config.pipeline.fu_config);

        Self {
            rob: Rob::new(rob_size),
            store_buffer: StoreBuffer::new(config.pipeline.store_buffer_size),
            load_queue: LoadQueue::new(config.pipeline.load_queue_size),
            prf,
            free_list: FreeList::new(prf_total, num_arch),
            rename_map: RenameMap::new(),
            committed_rename_map: RenameMap::new(),
            issue_queue: IssueQueue::new(config.pipeline.issue_queue_size),
            fu_pool,
            pending_results: Vec::new(),
            width: config.pipeline.width,
            rename_width: config.pipeline.rename_width(),
            issue_width: config.pipeline.issue_width(),
            commit_width: config.pipeline.commit_width(),
            load_ports: config.pipeline.load_ports,
            store_ports: config.pipeline.store_ports,
            execute_mem1: Vec::with_capacity(config.pipeline.width),
            mem1_mem2: Vec::with_capacity(config.pipeline.width),
            mem2_wb: Vec::with_capacity(config.pipeline.width),
            cycle: 0,
            mdp: MemDepUnit::new(config),
            checkpoints: CheckpointTable::new(config.pipeline.checkpoint_count),
            squash_stall_remaining: 0,
            serialization: serialize::Serialization::Off,
            redirect_latency: config.pipeline.redirect_latency(),
            vec_prf: {
                let prf_vpr_size = config.pipeline.prf_vpr_size;
                let vlen = Vlen::new_unchecked(config.pipeline.vlen);
                let mut vprf = VecPhysRegFile::new(prf_vpr_size, vlen);
                vprf.mark_arch_ready(32); // identity-mapped arch slots 0..31
                vprf
            },
            vec_free_list: FreeList::new(config.pipeline.prf_vpr_size, 32),
            vec_pending: Vec::new(),
            num_vec_lanes: NumLanes::new(
                config.pipeline.num_vec_lanes.unwrap_or_else(|| (config.pipeline.vlen / 64).max(1)),
            ),
            vec_mem_pending: std::collections::VecDeque::new(),
            vec_mem_inflight: Vec::new(),
            vec_store_buffer: VecStoreBuffer::new(
                config.pipeline.vec_store_buffer_size,
                config.pipeline.vec_store_forwarding,
            ),
            common: crate::core::pipeline::engine::BackendCommon {
                pipeline_id,
                l1_i_id,
                l1_d_id,
                ..crate::core::pipeline::engine::BackendCommon::default()
            },
        }
    }

    /// Copy initial architectural register values into the identity-mapped PRF slots.
    ///
    /// Must be called after CPU register init but before the first pipeline tick.
    pub fn sync_arch_regs(&mut self, state: &crate::sim::CoreCtx<'_>) {
        use crate::common::RegIdx;
        use crate::core::pipeline::prf::PhysReg;
        use crate::core::units::vpu::types::VRegIdx;
        for i in 1u8..32 {
            let val = state.hart.regs.read(RegIdx::new(i));
            if val != 0 {
                self.prf.write(PhysReg(i as u16), val);
            }
        }
        for i in 0u8..32 {
            let val = state.hart.regs.read_f(RegIdx::new(i));
            if val != 0 {
                self.prf.write(PhysReg((32 + i) as u16), val);
            }
        }
        for i in 0u8..32 {
            let vreg = VRegIdx::new(i);
            let bytes = state.hart.regs.vpr().read_bytes(vreg);
            self.vec_prf.write_bytes(VecPhysReg::new(i as u16), bytes);
        }
    }

    /// Squash stall penalty: ROB has `width` read ports for reclaim + rename rebuild.
    fn compute_squash_stall(&self, squashed: usize, surviving: usize) -> u64 {
        let w = self.width.max(1);
        let squash_cycles = squashed.div_ceil(w).saturating_sub(1);
        let rebuild_cycles = surviving.div_ceil(w);
        (squash_cycles + rebuild_cycles) as u64
    }

    /// Rebuild the speculative rename map after a partial flush by replaying surviving ROB entries.
    fn rebuild_rename_map(&mut self) {
        self.rename_map = self.committed_rename_map.clone();
        for entry in self.rob.iter_in_order() {
            if entry.ctrl.reg_write && !entry.rd.is_zero() {
                self.rename_map.set(entry.rd, false, entry.phys_dst);
            } else if entry.ctrl.fp_reg_write {
                self.rename_map.set(entry.rd, true, entry.phys_dst);
            }
            if entry.vec_dst_count > 0 {
                let vd_base = entry.ctrl.vd.as_u8();
                for i in 0..entry.vec_dst_count as usize {
                    let vreg = VRegIdx::new(vd_base + i as u8);
                    self.rename_map.set_vec(vreg, entry.vec_phys_dst[i]);
                }
            }
        }
    }

    /// Takes a squash: drops everything younger than its kept tag (or the
    /// whole window when that tag has already retired), reclaims their
    /// physical registers, restores the rename map, and redirects fetch.
    fn apply_squash(
        &mut self,
        state: &mut CoreCtx<'_>,
        squash: PendingSquash,
        redirect: &mut Option<u64>,
    ) {
        let paths = &state.core.stat_paths.pipeline;
        state.shared.stats.counter(paths.stalls_control).inc();
        state.shared.stats.counter(paths.flushes_total).inc();
        match squash.redirect.cause {
            SquashCause::Branch => state.shared.stats.counter(paths.flushes_branch).inc(),
            SquashCause::System => state.shared.stats.counter(paths.flushes_system).inc(),
            SquashCause::MemoryOrder | SquashCause::Coherence => {}
        }

        self.serialization.squash(|tag| squash.squashes(tag));
        let keep_tag = squash.keep_tag.filter(|tag| self.rob.find_entry(*tag).is_some());
        let keep_seq = keep_tag.and_then(|tag| self.rob.find_entry(tag)).map(|entry| entry.seq);
        let squashed = if let Some(keep_tag) = keep_tag {
            for entry in self.rob.iter_after(keep_tag) {
                self.free_list.reclaim(entry.phys_dst);
                for i in 0..entry.vec_dst_count as usize {
                    self.vec_free_list.reclaim(entry.vec_phys_dst[i]);
                }
            }
            self.rob.iter_after(keep_tag).count()
        } else {
            for entry in self.rob.iter_all() {
                self.free_list.reclaim(entry.phys_dst);
                for i in 0..entry.vec_dst_count as usize {
                    self.vec_free_list.reclaim(entry.vec_phys_dst[i]);
                }
            }
            self.rob.len()
        };
        state.shared.stats.counter(paths.flushes_squashed_insns).add(squashed as u64);

        if let Some(keep_tag) = keep_tag {
            // flush_after, not flush: older un-issued IQ entries must survive or deadlock the pipeline.
            self.issue_queue.flush_after(keep_tag);
            self.rob.flush_after(keep_tag);
            self.store_buffer.flush_after(keep_tag);
            self.load_queue.flush_after(keep_tag);
            self.mdp.flush_after(keep_tag, &self.rob);
            self.vec_store_buffer.flush_after(keep_tag);
            self.common.squash_after(keep_tag);
        } else {
            self.issue_queue.flush();
            self.rob.flush_all();
            self.store_buffer.flush_speculative();
            self.load_queue.flush();
            self.mdp.flush();
            self.vec_store_buffer.flush_all();
            self.common.squash_all();
        }
        let survives = |tag: crate::core::pipeline::rob::RobTag| {
            keep_tag.is_some_and(|keep_tag| tag.is_older_or_eq(keep_tag))
        };
        self.mem1_mem2.retain(|e| survives(e.rob_tag));
        self.mem2_wb.retain(|e| survives(e.rob_tag));
        self.pending_results.retain(|p| survives(p.entry.rob_tag));
        self.vec_pending.retain(|v| survives(v.rob_tag));
        self.vec_mem_pending.retain(|m| survives(m.entry.rob_tag));
        self.vec_mem_inflight.retain(|m| survives(m.rob_tag));
        self.execute_mem1.retain(|e| survives(e.rob_tag));

        // Restore speculative rename map: checkpoint (O(1)) or forward ROB walk rebuild.
        let surviving = self.rob.len();
        let checkpoint = keep_tag
            .filter(|_| self.checkpoints.capacity() > 0)
            .and_then(|tag| self.checkpoints.find_by_tag(tag).map(|ckpt| ckpt.rename_map.clone()));
        if let Some(rename_map) = checkpoint {
            self.rename_map = rename_map;
            self.squash_stall_remaining = self.compute_squash_stall(squashed, 0);
        } else {
            self.rebuild_rename_map();
            self.squash_stall_remaining = self.compute_squash_stall(squashed, surviving);
            state
                .shared
                .stats
                .counter(paths.stalls_rename_rebuild)
                .add(surviving.div_ceil(self.width.max(1)) as u64);
        }
        if let Some(keep_tag) = keep_tag {
            self.checkpoints.flush_after(keep_tag);
        } else {
            self.checkpoints.flush_all();
        }

        *redirect = Some(squash.redirect.target);
        let now = state.cycle;
        self.common.squash_predictions(&mut state.core.branch_predictor, &squash, keep_seq, now);
    }

    /// Pump pending vec mem element micro-ops into `vec_mem_pending`, bounded by LQ capacity.
    fn issue_vec_mem_waves(&mut self) {
        for inflight in &mut self.vec_mem_inflight {
            while let Some(front) = inflight.pending_micro_ops.front() {
                if !front.is_store {
                    let w = mem_width_from_eew_bytes(front.eew.bytes());
                    if !self.load_queue.allocate(front.entry.rob_tag, w, Some(front.elem_idx)) {
                        break;
                    }
                }
                let Some(mop) = inflight.pending_micro_ops.pop_front() else { break };
                self.vec_mem_pending.push_back(mop);
            }
        }
    }
}

impl ExecutionEngine for O3Engine {
    fn tick(
        &mut self,
        state: &mut CoreCtx<'_>,
        rename_output: &mut Vec<RenameIssueEntry>,
        redirect: &mut Option<u64>,
    ) {
        self.cycle += 1;
        let now = self.cycle;

        // Squash recovery: ROB read ports are busy with reclaim / rename rebuild.
        if self.squash_stall_remaining > 0 {
            self.squash_stall_remaining -= 1;
            state.shared.stats.counter(state.core.stat_paths.pipeline.stalls_squash).inc();
        }

        if let Some(squash) = self.common.take_due_squash(now) {
            self.apply_squash(state, squash, redirect);
            rename_output.clear();
        }

        let commit_event = commit::commit_stage(
            state,
            CommitResources {
                common: &mut self.common,
                rob: &mut self.rob,
                store_buffer: &mut self.store_buffer,
                vec_store_buffer: &mut self.vec_store_buffer,
                width: self.commit_width,
                registers: CommitRegisters::Renamed {
                    rename_map: &mut self.committed_rename_map,
                    free_list: &mut self.free_list,
                    prf: &mut self.prf,
                    load_queue: &mut self.load_queue,
                    checkpoints: &mut self.checkpoints,
                    vec_prf: &mut self.vec_prf,
                    vec_free_list: &mut self.vec_free_list,
                },
            },
        );

        match commit_event {
            Some(CommitEvent::Trap(trap, pc)) => {
                // Full flush: committed_rename_map is used directly, no rebuild.
                let squashed = self.rob.len();
                self.flush(state);
                self.squash_stall_remaining = self.compute_squash_stall(squashed, 0);
                state.trap(&trap, pc);
                *redirect = Some(state.hart.pc);
                return;
            }
            Some(CommitEvent::ReExecute(pc) | CommitEvent::SquashAfter(pc)) => {
                let squashed = self.rob.len();
                self.flush(state);
                self.squash_stall_remaining = self.compute_squash_stall(squashed, 0);
                state.hart.pc = pc;
                *redirect = Some(pc);
                return;
            }
            None => {}
        }
        self.serialization.observe(self.rob.is_empty(), now);

        // Intercept vec mem micro-ops before the normal writeback stage.
        {
            let mut scalar_wb = Vec::with_capacity(self.mem2_wb.len());
            let vec_entries = std::mem::take(&mut self.mem2_wb);
            for wb in vec_entries {
                if let Some(ref vme) = wb.vec_mem {
                    let retired =
                        retire_element(&wb, vme, &mut self.vec_mem_inflight, &mut self.rob);
                    if retired.write_data {
                        let vlen_bits = self.vec_prf.vlen().bits();
                        let eew_bits = vme.eew.bytes() * 8;
                        let elems_per_reg = if eew_bits > 0 { vlen_bits / eew_bits } else { 1 };
                        let local = ElemIdx::new(vme.elem_idx.as_usize() % elems_per_reg);
                        self.vec_prf.write_element(vme.vd_phys, local, vme.eew, wb.load_data);
                    }
                    if !vme.is_store {
                        self.load_queue.deallocate_elem(wb.rob_tag, vme.elem_idx);
                    }
                    // Fire chaining wakeup only on full completion: dependents bulk-read all elements.
                    if retired.completed
                        && let Some(parent) =
                            self.vec_mem_inflight.iter_mut().find(|m| m.rob_tag == wb.rob_tag)
                        && !parent.wakeup_fired
                    {
                        for j in 0..parent.vd_count as usize {
                            self.vec_prf.mark_ready(parent.vd_phys[j]);
                        }
                        for j in 0..parent.vd_count as usize {
                            self.issue_queue.wakeup_vec_phys(parent.vd_phys[j], &self.vec_prf);
                        }
                        parent.wakeup_fired = true;
                    }
                } else {
                    scalar_wb.push(wb);
                }
            }
            self.mem2_wb = scalar_wb;
        }

        // Snapshot completing wakeups before writeback so dependents can wake via PRF.
        let wb_wakeups: Vec<_> = self
            .mem2_wb
            .iter()
            .filter(|wb| wb.trap.is_none())
            .map(|wb| {
                let val = if wb.ctrl.mem_read {
                    wb.load_data
                } else if wb.ctrl.control_flow == ControlFlow::Jump {
                    wb.pc.wrapping_add(wb.inst_size.as_u64())
                } else {
                    wb.alu
                };
                (wb.rob_tag, wb.rd_phys, val)
            })
            .collect();

        writeback::writeback_stage(&mut state.stage(), &mut self.mem2_wb, &mut self.rob);

        for (_tag, rd_phys, val) in &wb_wakeups {
            self.prf.write(*rd_phys, *val);
            self.issue_queue.wakeup_phys(*rd_phys, *val);
        }

        let wb_before = self.mem2_wb.len();
        let mem_violation = memory2::memory2_stage(
            &mut state.stage(),
            &mut self.mem1_mem2,
            &mut self.mem2_wb,
            &mut self.store_buffer,
            Some(&mut self.load_queue),
            Some(&mut self.vec_store_buffer),
        );

        // Stores that resolve in memory2: store-conditionals, AMOs, vectors.
        for entry in &self.mem2_wb[wb_before..] {
            if entry.ctrl.mem_write
                && (entry.ctrl.atomic_op != AtomicOp::None || entry.vec_mem.is_some())
                && let Some(store_tag) = self.mdp.store_resolved(entry.rob_tag)
            {
                self.issue_queue.wakeup_mem_dep(&[store_tag]);
            }
        }

        // Packet-based memory1 always accepts work and parks loads in
        // `common.outstanding_loads`. Backpressure comes from the L1D's
        // pending table when the cache is saturated, which surfaces as
        // mailbox-drain backlogs rather than a per-engine `mem1_busy` gate.
        let mut input = std::mem::take(&mut self.execute_mem1);
        let resolved = memory1::memory1_stage(&mut state.stage(), self, &mut input);
        self.execute_mem1.extend(input);
        for store_tag in resolved.resolved_stores {
            if let Some(tag) = self.mdp.store_resolved(store_tag) {
                self.issue_queue.wakeup_mem_dep(&[tag]);
            }
        }
        let mem_violation = match (mem_violation, resolved.violation) {
            (Some(m2), Some(m1)) if m1.0.is_older_than(m2.0) => Some(m1),
            (Some(m2), _) => Some(m2),
            (None, m1) => m1,
        };

        let squash = match (mem_violation, self.common.coherence_violation.take()) {
            (Some((tag, _)), Some(coherence_tag)) if coherence_tag.is_older_than(tag) => {
                Some((coherence_tag, None))
            }
            (Some((tag, store_pc)), _) => Some((tag, Some(store_pc))),
            (None, Some(coherence_tag)) => Some((coherence_tag, None)),
            (None, None) => None,
        };

        if let Some((violating_tag, store_pc)) = squash {
            let violation_pc = self.rob.find_entry(violating_tag).map_or(state.hart.pc, |e| e.pc);
            let cause = if let Some(store_pc) = store_pc {
                self.mdp.violation(violation_pc, store_pc);
                state
                    .shared
                    .stats
                    .counter(state.core.stat_paths.pipeline.flushes_mem_violations)
                    .inc();
                SquashCause::MemoryOrder
            } else {
                state.shared.stats.counter(state.core.stat_paths.lsq.coherence_violations).inc();
                SquashCause::Coherence
            };
            // The violating load re-executes, so it does not survive either.
            self.common.request_squash(PendingSquash {
                keep_tag: self.rob.prev_tag_of(violating_tag),
                redirect: Redirect::to(violation_pc, cause),
                apply_at: now + self.redirect_latency,
            });
        }

        let _ = now;

        // Backpressure only while memory1 holds ops behind an unresolved
        // translation walk. Ops waiting on a store-buffer drain live in
        // `common.mem1_replay` and never gate issue.
        let mem_backpressured = !self.execute_mem1.is_empty();

        if mem_backpressured {
            state.shared.stats.counter(state.core.stat_paths.pipeline.stalls_backpressure).inc();
        }

        {
            let mut i = 0;
            while i < self.pending_results.len() {
                if self.pending_results[i].complete_cycle <= now {
                    let pr = self.pending_results.swap_remove(i);
                    let entry = pr.entry;
                    let fu_type = pr.fu_type;

                    state
                        .shared
                        .stats
                        .counter(state.core.stat_paths.fu.all[fu_type as usize])
                        .inc();

                    if entry.ctrl.mem_read
                        || entry.ctrl.mem_write
                        || entry.ctrl.atomic_op != crate::core::pipeline::signals::AtomicOp::None
                    {
                        self.execute_mem1.push(entry);
                    } else if let Some(trap) = entry.trap {
                        let stage =
                            entry.exception_stage.unwrap_or(crate::common::ExceptionStage::Execute);
                        self.rob.fault(entry.rob_tag, trap, stage);
                    } else {
                        let val = if entry.ctrl.control_flow == ControlFlow::Jump {
                            entry.pc.wrapping_add(entry.inst_size.as_u64())
                        } else {
                            entry.alu
                        };
                        if entry.fp_flags != 0 {
                            self.rob.set_fp_flags(entry.rob_tag, entry.fp_flags);
                        }
                        if let Some(info) = entry.sfence_vma {
                            self.rob.set_sfence_vma(entry.rob_tag, info);
                        }
                        // CSR writes deferred to commit so speculative state isn't observed on trap.
                        self.rob.complete(entry.rob_tag, val);
                        self.prf.write(entry.rd_phys, val);
                        self.issue_queue.wakeup_phys(entry.rd_phys, val);
                    }
                } else {
                    i += 1;
                }
            }
        }

        // Refill vec_mem_pending from in-flight vec mem ops, then drain to execute_mem1.
        self.issue_vec_mem_waves();
        {
            let mut loads_issued = 0usize;
            let mut stores_issued = 0usize;
            while let Some(front) = self.vec_mem_pending.front() {
                if front.is_store {
                    if stores_issued >= self.store_ports {
                        break;
                    }
                    stores_issued += 1;
                } else {
                    if loads_issued >= self.load_ports {
                        break;
                    }
                    loads_issued += 1;
                }
                let Some(mop) = self.vec_mem_pending.pop_front() else { break };
                self.execute_mem1.push(mop.entry);
            }
        }

        {
            let mut i = 0;
            while i < self.vec_pending.len() {
                let vp = &mut self.vec_pending[i];
                if !vp.wakeup_fired && now >= vp.first_group_ready {
                    for j in 0..vp.vd_count as usize {
                        self.vec_prf.mark_ready(vp.vd_phys[j]);
                    }
                    for j in 0..vp.vd_count as usize {
                        self.issue_queue.wakeup_vec_phys(vp.vd_phys[j], &self.vec_prf);
                    }
                    vp.wakeup_fired = true;
                }
                // Some vl=0 ops reach full_complete before first_group_ready; wake here too.
                if now >= vp.full_complete {
                    if !vp.wakeup_fired {
                        for j in 0..vp.vd_count as usize {
                            self.vec_prf.mark_ready(vp.vd_phys[j]);
                        }
                        for j in 0..vp.vd_count as usize {
                            self.issue_queue.wakeup_vec_phys(vp.vd_phys[j], &self.vec_prf);
                        }
                        vp.wakeup_fired = true;
                    }
                    self.rob.complete(vp.rob_tag, 0);
                    let _ = self.vec_pending.swap_remove(i);
                } else {
                    i += 1;
                }
            }
        }

        {
            let issued = self.issue_queue.select(
                self.issue_width,
                &self.store_buffer,
                &self.rob,
                self.load_ports,
                self.store_ports,
            );

            let mut issued_count = 0;
            let mut stalled_fu = false;

            for selected in issued {
                let entry = selected.entry;
                let mem_dep = selected.mem_dep;
                let fu_type = FuType::classify(&entry.ctrl);
                let rob_tag = entry.rob_tag;
                let is_mem_instr = entry.ctrl.mem_read || entry.ctrl.mem_write;

                // Backpressure blocks memory ops only; ALU/branch continue freely.
                if mem_backpressured && fu_type == FuType::Mem {
                    let ok = self.issue_queue.dispatch(
                        entry,
                        &self.rob,
                        &state.stage(),
                        Some(&self.prf),
                        Some(&self.vec_prf),
                        mem_dep,
                    );
                    debug_assert!(ok, "re-dispatch after mem backpressure failed");
                    continue;
                }

                let Some(unit) = self.fu_pool.free_unit(fu_type, now) else {
                    state
                        .shared
                        .stats
                        .counter(state.core.stat_paths.pipeline.stalls_fu_structural)
                        .inc();
                    stalled_fu = true;
                    let ok = self.issue_queue.dispatch(
                        entry,
                        &self.rob,
                        &state.stage(),
                        Some(&self.prf),
                        Some(&self.vec_prf),
                        mem_dep,
                    );
                    debug_assert!(ok, "re-dispatch after FU stall failed");
                    continue;
                };

                if is_mem_instr {
                    self.mdp.issued(rob_tag);
                }

                // vsetvl* run synchronously in execute_one; exclude from deferred VecPrfView.
                let is_vec_config = entry.ctrl.vec_op.is_config();
                let is_vec_non_mem =
                    fu_type.is_vector() && fu_type != FuType::VecMem && !is_vec_config;
                let is_vec_mem_op = fu_type == FuType::VecMem;

                // For vec mem ops, override vd group from vec_mem_dst_count (nf × EMUL_data).
                let mut vec_grp = entry.ctrl.vec_op.operand_groups(
                    entry.ctrl.vec_lmul_regs,
                    entry.ctrl.vec_lmul_is_fractional,
                    entry.ctrl.vec_src_encoding,
                    entry.ctrl.vec_nf,
                    entry.ctrl.vec_broadcast_vs2,
                );
                if is_vec_mem_op {
                    let vtype = crate::core::units::vpu::types::parse_vtype(entry.vec_vtype);
                    if !vtype.vill {
                        vec_grp.vd = crate::core::units::vpu::mem::vec_mem_dst_count(
                            entry.ctrl.vec_op,
                            entry.ctrl.vec_eew,
                            vtype.vsew,
                            vtype.vlmul,
                            entry.ctrl.vec_nf,
                        );
                    }
                }
                let vec_dst_info = if entry.ctrl.vec_reg_write && vec_grp.vd > 0 {
                    Some((entry.vd_phys, vec_grp.vd, entry.ctrl.vd))
                } else {
                    None
                };

                let complete_cycle = if is_vec_non_mem {
                    use crate::core::pipeline::signals::VectorOp;
                    use crate::core::units::vpu::lane_model;
                    use crate::core::units::vpu::reduction;

                    let vl = entry.vec_vl as usize;
                    let startup = self.fu_pool.startup_latency(fu_type);
                    let pipelined = self.fu_pool.is_pipelined(fu_type);
                    let vec_op = entry.ctrl.vec_op;

                    let lanes = self.num_vec_lanes.as_usize();

                    let latency = if reduction::is_reduction(vec_op) {
                        // Ordered FP reductions are sequential; others use the tree model.
                        let is_ordered =
                            matches!(vec_op, VectorOp::VFRedOSum | VectorOp::VFWRedOSum);
                        lane_model::compute_reduction_latency(vl, lanes, startup, is_ordered)
                    } else if fu_type == FuType::VecPermute {
                        let groups = (vl.div_ceil(lanes)) as u64;
                        let base_latency = match vec_op {
                            VectorOp::VRgather | VectorOp::VRgatherEi16
                                if entry.ctrl.vec_src_encoding
                                    == crate::core::pipeline::signals::VecSrcEncoding::VV =>
                            {
                                startup + groups.saturating_mul(2).saturating_sub(1)
                            }
                            VectorOp::VRgather | VectorOp::VRgatherEi16 => {
                                startup + groups.saturating_sub(1)
                            }
                            VectorOp::VCompress => {
                                startup + groups.saturating_mul(2).saturating_sub(1)
                            }
                            _ => lane_model::compute_vec_latency(vl, lanes, startup, pipelined),
                        };
                        base_latency.max(1)
                    } else {
                        lane_model::compute_vec_latency(vl, lanes, startup, pipelined)
                    };
                    self.fu_pool.acquire_with_latency(unit, now, latency)
                } else {
                    // A vector memory op's unit is the address generator;
                    // its elements pay their latency in memory1 and memory2.
                    self.fu_pool.acquire(unit, now)
                };

                let (ex_result, redirect) =
                    execute::execute_one(&mut state.stage(), &entry, &mut self.rob);
                issued_count += 1;
                if let Some(redirect) = redirect {
                    self.common.request_squash(PendingSquash {
                        keep_tag: Some(ex_result.rob_tag),
                        redirect,
                        apply_at: complete_cycle + self.redirect_latency,
                    });
                }
                if is_vec_config {
                    self.common.vector_config_unresolved = false;
                }

                if is_vec_non_mem && ex_result.trap.is_none() {
                    use crate::core::units::vpu::execute::execute_vec_op_on;
                    use crate::core::units::vpu::lane_model;

                    // Build arch→phys mapping from rename-time physregs so later renames don't alias.
                    let mut mapping = [VecPhysReg::ZERO; 32];
                    for i in 0..32u8 {
                        mapping[i as usize] = self.rename_map.get_vec(VRegIdx::new(i));
                    }

                    {
                        let base = entry.ctrl.vs2.as_u8() as usize;
                        for i in 0..entry.vec_src2_count as usize {
                            if base + i < 32 {
                                mapping[base + i] = entry.vs2_phys[i];
                            }
                        }
                    }
                    {
                        let base = entry.ctrl.vs1.as_u8() as usize;
                        for i in 0..entry.vec_src1_count as usize {
                            if base + i < 32 {
                                mapping[base + i] = entry.vs1_phys[i];
                            }
                        }
                    }
                    if let Some((vd_phys_arr, vd_cnt, vd_reg)) = vec_dst_info {
                        let base = vd_reg.as_u8() as usize;
                        for i in 0..vd_cnt as usize {
                            if base + i < 32 {
                                // Pre-copy old vd so tail/mask-undisturbed reads see correct baseline.
                                if i < entry.vec_src3_count as usize {
                                    self.vec_prf.copy_reg(vd_phys_arr[i], entry.vs3_phys[i]);
                                }
                                mapping[base + i] = vd_phys_arr[i];
                            }
                        }
                    }
                    if !entry.mask_phys.is_zero() {
                        mapping[0] = entry.mask_phys;
                    }

                    // Use dispatch-time CSR snapshot so in-flight vsetvl can't corrupt vtype/vl.
                    let vec_result_or_trap = {
                        let mut view = VecPrfView::new(&mut self.vec_prf, mapping);
                        execute_vec_op_on(
                            &mut view,
                            entry.vec_vtype,
                            entry.vec_vl,
                            entry.vec_vstart,
                            entry.vec_vxrm,
                            entry.vec_frm,
                            state.config.isa.vector.elen,
                            state.config.isa.vector.zvfh,
                            &entry,
                        )
                    };

                    let vec_result = match vec_result_or_trap {
                        Ok(r) => r,
                        Err(trap) => {
                            self.rob.fault(
                                ex_result.rob_tag,
                                trap,
                                crate::common::error::ExceptionStage::Execute,
                            );
                            continue;
                        }
                    };

                    if vec_result.fp_flags != 0 {
                        self.rob.set_fp_flags(ex_result.rob_tag, vec_result.fp_flags);
                    }
                    if vec_result.vxsat {
                        self.rob.set_vxsat(ex_result.rob_tag, true);
                    }

                    let startup = self.fu_pool.startup_latency(fu_type);
                    let first_ready = lane_model::first_group_ready(now, startup);

                    // Scalar-result vec ops (vmv.x.s, vcpop.m, vfirst.m) take the scalar path.
                    if ex_result.ctrl.vec_reg_write {
                        let vd_count = vec_dst_info.map_or(0u8, |(_, c, _)| c);
                        let vd_phys_arr = vec_dst_info.map_or([VecPhysReg::ZERO; 8], |(p, _, _)| p);
                        self.vec_pending.push(VecPendingResult {
                            rob_tag: ex_result.rob_tag,
                            vd_phys: vd_phys_arr,
                            vd_count,
                            first_group_ready: first_ready,
                            full_complete: complete_cycle,
                            wakeup_fired: false,
                        });
                    } else {
                        let mut scalar_result_entry = ex_result.clone();
                        scalar_result_entry.alu = vec_result.scalar_result;
                        self.pending_results.push(PendingResult {
                            entry: scalar_result_entry,
                            complete_cycle,
                            fu_type,
                        });
                    }

                    continue;
                }

                if is_vec_mem_op && ex_result.trap.is_none() {
                    let vec_op = ex_result.ctrl.vec_op;
                    let is_store = is_vec_store(vec_op);
                    let vd_count = vec_dst_info.map_or(0u8, |(_, c, _)| c);
                    let vd_phys_arr = vec_dst_info.map_or([VecPhysReg::ZERO; 8], |(p, _, _)| p);

                    // Reject illegal EMUL (>8) before generate_element_addrs_vrf would panic.
                    let vtype = crate::core::units::vpu::types::parse_vtype(entry.vec_vtype);
                    if let Err(trap) = crate::core::units::vpu::mem::check_vec_mem_emul(
                        ex_result.inst,
                        vec_op,
                        &entry.ctrl,
                        &vtype,
                    ) {
                        self.rob.fault(
                            ex_result.rob_tag,
                            trap,
                            crate::common::error::ExceptionStage::Execute,
                        );
                        continue;
                    }

                    let mut mapping = [VecPhysReg::ZERO; 32];
                    for i in 0..32u8 {
                        mapping[i as usize] = self.rename_map.get_vec(VRegIdx::new(i));
                    }
                    {
                        let base = entry.ctrl.vs2.as_u8() as usize;
                        for i in 0..entry.vec_src2_count as usize {
                            if base + i < 32 {
                                mapping[base + i] = entry.vs2_phys[i];
                            }
                        }
                    }
                    {
                        let base = entry.ctrl.vd.as_u8() as usize;
                        for i in 0..entry.vec_src3_count as usize {
                            if base + i < 32 {
                                mapping[base + i] = entry.vs3_phys[i];
                            }
                        }
                    }
                    if !entry.mask_phys.is_zero() {
                        mapping[0] = entry.mask_phys;
                    }
                    let micro_ops = {
                        let view = VecPrfView::new(&mut self.vec_prf, mapping);
                        generate_element_addrs_vrf(
                            &view,
                            ex_result.alu,
                            ex_result.store_data as i64,
                            &entry.ctrl,
                            entry.vec_vtype,
                            entry.vec_vl as usize,
                            entry.vec_vstart as usize,
                            vec_op,
                            &vd_phys_arr,
                            vd_count,
                        )
                    };

                    // Pre-copy old vd so tail / mask-undisturbed elements observe prior values.
                    if !is_store && let Some((vd_phys_arr_pre, vd_cnt_pre, _)) = vec_dst_info {
                        let copy_count = (vd_cnt_pre as usize).min(entry.vec_src3_count as usize);
                        for (i, &dst) in vd_phys_arr_pre.iter().enumerate().take(copy_count) {
                            self.vec_prf.copy_reg(dst, entry.vs3_phys[i]);
                        }
                    }

                    if micro_ops.is_empty() {
                        // VL=0 / vill=1: route through vec_pending so destination physregs surface ready.
                        let startup = self.fu_pool.startup_latency(fu_type);
                        let first_ready =
                            crate::core::units::vpu::lane_model::first_group_ready(now, startup);
                        self.vec_pending.push(VecPendingResult {
                            rob_tag: ex_result.rob_tag,
                            vd_phys: vd_phys_arr,
                            vd_count,
                            first_group_ready: first_ready,
                            full_complete: complete_cycle,
                            wakeup_fired: false,
                        });
                    } else {
                        // Build all micro-ops up front; issue_vec_mem_waves releases them in waves.
                        let total = micro_ops.len();
                        let all_micro_ops = micro_ops_for(&ex_result, micro_ops, is_store);

                        if is_store {
                            self.vec_store_buffer.set_expected_elements(ex_result.rob_tag, total);
                        }

                        self.vec_mem_inflight.push(VecMemInflight {
                            rob_tag: ex_result.rob_tag,
                            remaining: total,
                            vd_phys: vd_phys_arr,
                            vd_count,
                            wakeup_fired: false,
                            pending_micro_ops: all_micro_ops,
                            trimmed_at: None,
                        });
                    }

                    continue;
                }

                self.pending_results.push(PendingResult {
                    entry: ex_result,
                    complete_cycle,
                    fu_type,
                });
            }

            if issued_count == 0 && !stalled_fu && !self.issue_queue.is_empty() {
                state.shared.stats.counter(state.core.stat_paths.pipeline.stalls_data).inc();
            }
        }

        {
            let entries = std::mem::take(rename_output);
            for entry in entries {
                let is_load = entry.ctrl.mem_read;
                let is_store = entry.ctrl.mem_write;
                let is_atomic =
                    entry.ctrl.atomic_op != crate::core::pipeline::signals::AtomicOp::None;
                let mem_dep =
                    self.mdp.dispatch(entry.pc, entry.rob_tag, is_load, is_store, is_atomic);
                let ok = self.issue_queue.dispatch(
                    entry,
                    &self.rob,
                    &state.stage(),
                    Some(&self.prf),
                    Some(&self.vec_prf),
                    mem_dep,
                );
                debug_assert!(ok, "IQ dispatch failed — rename budget should prevent this");
            }
        }

        let mdp_stats = self.mdp.stats();
        {
            let bypass = state.shared.stats.counter(state.core.stat_paths.mdp.predictions_bypass);
            bypass.reset();
            bypass.add(mdp_stats.predictions_bypass);
        }
        {
            let wait_all =
                state.shared.stats.counter(state.core.stat_paths.mdp.predictions_wait_all);
            wait_all.reset();
            wait_all.add(mdp_stats.predictions_wait_all);
        }
        {
            let wait_for =
                state.shared.stats.counter(state.core.stat_paths.mdp.predictions_wait_for);
            wait_for.reset();
            wait_for.add(mdp_stats.predictions_wait_for);
        }
        {
            let violations = state.shared.stats.counter(state.core.stat_paths.mdp.violations);
            violations.reset();
            violations.add(mdp_stats.violations);
        }
    }

    fn can_accept(&self) -> usize {
        // Squash recovery monopolises ROB read ports; rename can't dispatch during it.
        if self.squash_stall_remaining > 0 {
            return 0;
        }
        let rob_free = self.rob.free_slots();
        let sb_free = self.store_buffer.free_slots();
        let vsb_free = self.vec_store_buffer.free_slots();
        let lq_free = self.load_queue.free_slots();
        let iq_free = self.issue_queue.available_slots();
        let prf_free = self.free_list.available();
        let vec_prf_free = self.vec_free_list.available();
        rob_free
            .min(sb_free)
            .min(vsb_free)
            .min(lq_free)
            .min(iq_free)
            .min(prf_free)
            .min(vec_prf_free)
            .min(self.rename_width)
    }

    fn flush(&mut self, state: &mut CoreCtx<'_>) {
        self.serialization = serialize::Serialization::Off;
        // Drain committed VSB writes; trap-driven flushes still owe pre-trap retired stores.
        self.vec_store_buffer.drain_all_committed(state, &mut self.common);

        for entry in self.rob.iter_all() {
            self.free_list.reclaim(entry.phys_dst);
            for i in 0..entry.vec_dst_count as usize {
                self.vec_free_list.reclaim(entry.vec_phys_dst[i]);
            }
        }
        self.rename_map = self.committed_rename_map.clone();

        self.rob.flush_all();
        self.store_buffer.flush_speculative();
        self.load_queue.flush();
        self.issue_queue.flush();
        self.mdp.flush();
        self.checkpoints.flush_all();
        // Caller sets squash_stall_remaining after this; not cleared here.
        self.pending_results.clear();
        self.vec_pending.clear();
        self.vec_mem_pending.clear();
        self.vec_mem_inflight.clear();
        self.vec_store_buffer.flush_all();
        self.execute_mem1.clear();
        self.common.mem1_replay.clear();
        self.common.coherence_violation = None;
        self.common.pending_squash = None;
        self.mem1_mem2.clear();
        self.mem2_wb.clear();
        self.common.flush_predictions(&mut state.core.branch_predictor);

        // Conservation invariant: every phys reg is either free or held by the committed map.
        debug_assert_eq!(
            self.free_list.available() + 64,
            self.prf.capacity(),
            "PRF register leak detected: free={} + 64 mapped != {} total",
            self.free_list.available(),
            self.prf.capacity(),
        );
        debug_assert_eq!(
            self.vec_free_list.available() + 32,
            self.vec_prf.capacity(),
            "Vec PRF register leak detected: free={} + 32 mapped != {} total",
            self.vec_free_list.available(),
            self.vec_prf.capacity(),
        );
    }

    fn rob(&self) -> &Rob {
        &self.rob
    }

    fn store_buffer(&self) -> &StoreBuffer {
        &self.store_buffer
    }

    fn store_buffer_mut(&mut self) -> &mut StoreBuffer {
        &mut self.store_buffer
    }

    fn vec_store_buffer(&self) -> &VecStoreBuffer {
        &self.vec_store_buffer
    }

    fn rename(
        &mut self,
        state: &mut crate::sim::StageCtx<'_>,
        id: crate::core::pipeline::latches::IdExEntry,
    ) -> crate::core::pipeline::engine::Renamed {
        self.rename_one(state, id)
    }

    fn drain_committed_stores(&mut self, state: &mut CoreCtx<'_>) {
        commit::drain_all_committed(
            state,
            &mut self.common,
            &mut self.store_buffer,
            &mut self.vec_store_buffer,
        );
    }

    fn execute_mem1_mut(&mut self) -> &mut Vec<crate::core::pipeline::latches::ExMem1Entry> {
        &mut self.execute_mem1
    }

    fn mem1_mem2_mut(&mut self) -> &mut Vec<crate::core::pipeline::latches::Mem1Mem2Entry> {
        &mut self.mem1_mem2
    }

    fn common(&self) -> &crate::core::pipeline::engine::BackendCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut crate::core::pipeline::engine::BackendCommon {
        &mut self.common
    }

    fn load_queue_mut(&mut self) -> Option<&mut LoadQueue> {
        Some(&mut self.load_queue)
    }

    fn has_register_renaming(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::RegIdx;
    use crate::config::Config;

    #[test]
    fn test_o3_engine_new_and_flush() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        let mut engine = O3Engine::new(
            &config,
            crate::sim::components::PipelineId::new(0),
            crate::sim::components::CacheId::new(0),
            crate::sim::components::CacheId::new(1),
        );
        assert_eq!(engine.width, config.pipeline.width);

        engine.flush(&mut state);
        assert_eq!(engine.execute_mem1.len(), 0);
    }

    #[test]
    fn test_o3_engine_sync_arch_regs() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let state = sys.core_ctx(0);
        let mut engine = O3Engine::new(
            &config,
            crate::sim::components::PipelineId::new(0),
            crate::sim::components::CacheId::new(0),
            crate::sim::components::CacheId::new(1),
        );

        state.hart.regs.write(RegIdx::new(1), 42);
        engine.sync_arch_regs(&state);

        assert_eq!(engine.prf.read(crate::core::pipeline::prf::PhysReg(1)), 42);
    }
}
