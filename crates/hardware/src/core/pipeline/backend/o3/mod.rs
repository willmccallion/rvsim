//! Out-of-Order (O3) backend: issue queue with wakeup/select, out-of-order execution.
//!
//! The O3 backend reuses shared pipeline stages (Memory1, Memory2, Writeback,
//! Commit) and shared hardware units (ALU, FPU, BRU), but has its own:
//! - **`IssueQueue`**: CAM-style with wakeup/select (vs FIFO for in-order)
//! - **`execute_one()`**: single-instruction execute (vs batch for in-order)

pub mod execute;
pub mod fu_pool;
pub mod issue_queue;

use crate::config::Config;
use crate::sim::SimState;
use crate::core::pipeline::backend::shared::{commit, memory1, memory2, writeback};
use crate::core::pipeline::checkpoint::CheckpointTable;
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::free_list::FreeList;
use crate::core::pipeline::latches::{
    ExMem1Entry, Mem1Mem2Entry, Mem2WbEntry, RenameIssueEntry, VecMemElement,
};
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::prf::{PhysReg, PhysRegFile};
use crate::core::pipeline::rename_map::RenameMap;
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::signals::{ControlFlow, VectorOp};
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::core::pipeline::vec_prf::VecPhysRegFile;
use crate::core::pipeline::vec_prf::VecPrfView;
use crate::core::pipeline::vec_store_buffer::VecStoreBuffer;
use crate::core::units::bru::BranchPredictor;
use crate::core::units::mdp::MemDepUnit;
use crate::core::units::vpu::chaining::VecPendingResult;
use crate::core::units::vpu::mem::{generate_element_addrs_vrf, is_vec_store};
use crate::core::units::vpu::types::{ElemIdx, NumLanes, VRegIdx, VecPhysReg, Vlen};

use self::fu_pool::{FuPool, FuType};
use self::issue_queue::IssueQueue;

/// A result that has been computed but not yet written back (pending due to latency).
#[derive(Debug)]
pub struct PendingResult {
    /// The execute-stage result entry.
    pub entry: ExMem1Entry,
    /// Cycle at which the result is ready (wakeup fires at this cycle).
    pub complete_cycle: u64,
    /// Functional unit type (for stats).
    pub fu_type: FuType,
    /// Whether the result has already been written to PRF (speculative wakeup).
    pub speculative_written: bool,
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
    /// Tag-based register scoreboard (kept for in-order compatibility; O3 uses PRF).
    pub scoreboard: Scoreboard,
    /// CAM-style issue queue with wakeup/select.
    pub issue_queue: IssueQueue,
    /// Functional unit pool for structural hazard modeling.
    pub fu_pool: FuPool,
    /// Results that have been computed but not yet written back.
    pub pending_results: Vec<PendingResult>,
    /// Pipeline width (max instructions issued/committed per cycle).
    pub width: usize,
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

/// A single vector memory micro-op representing one element (or cache-line chunk)
/// flowing through the Memory1 → Memory2 → Writeback pipeline.
#[derive(Debug, Clone)]
pub struct VecMemMicroOp {
    /// The `ExMem1Entry` carrying the element's virtual address and metadata.
    pub entry: ExMem1Entry,
    /// Element index within the vector register (for writeback targeting).
    pub elem_idx: crate::core::units::vpu::types::ElemIdx,
    /// Effective element width for this access.
    pub eew: crate::core::units::vpu::types::Sew,
    /// Destination physical vector register for this element's data.
    pub vd_phys: VecPhysReg,
    /// Whether this is a store (vs load).
    pub is_store: bool,
}

/// Per-element progress tracker for an in-flight vec mem op (data lives in `vec_store_buffer`).
#[derive(Debug, Clone)]
pub struct VecMemInflight {
    /// ROB tag of the parent vector memory instruction.
    pub rob_tag: crate::core::pipeline::rob::RobTag,
    /// Number of micro-ops still outstanding (not yet written back).
    pub remaining: usize,
    /// Physical destination registers for the LMUL group (for chaining wakeup).
    pub vd_phys: [VecPhysReg; 8],
    /// Number of destination registers in the LMUL group.
    pub vd_count: u8,
    /// Whether chaining wakeup has fired (first cache-line returned).
    pub wakeup_fired: bool,
    /// Micro-ops generated at issue but not yet pushed into the memory
    /// pipeline (waiting for LQ slots to free up).
    pub pending_micro_ops: std::collections::VecDeque<VecMemMicroOp>,
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
            scoreboard: Scoreboard::new(),
            issue_queue: IssueQueue::new(config.pipeline.issue_queue_size),
            fu_pool,
            pending_results: Vec::new(),
            width: config.pipeline.width,
            load_ports: config.pipeline.load_ports,
            store_ports: config.pipeline.store_ports,
            execute_mem1: Vec::with_capacity(config.pipeline.width),
            mem1_mem2: Vec::with_capacity(config.pipeline.width),
            mem2_wb: Vec::with_capacity(config.pipeline.width),
            cycle: 0,
            mdp: MemDepUnit::new(config),
            checkpoints: CheckpointTable::new(config.pipeline.checkpoint_count),
            squash_stall_remaining: 0,
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
            common: {
                let mut c = crate::core::pipeline::engine::BackendCommon::default();
                c.pipeline_id = pipeline_id;
                c.l1_i_id = l1_i_id;
                c.l1_d_id = l1_d_id;
                c
            },
        }
    }

    /// Copy initial architectural register values into the identity-mapped PRF slots.
    ///
    /// Must be called after CPU register init but before the first pipeline tick.
    pub fn sync_arch_regs(&mut self, cpu: &crate::sim::SimState) {
        use crate::common::RegIdx;
        use crate::core::pipeline::prf::PhysReg;
        use crate::core::units::vpu::types::VRegIdx;
        for i in 1u8..32 {
            let val = cpu.hart.regs.read(RegIdx::new(i));
            if val != 0 {
                self.prf.write(PhysReg(i as u16), val);
            }
        }
        for i in 0u8..32 {
            let val = cpu.hart.regs.read_f(RegIdx::new(i));
            if val != 0 {
                self.prf.write(PhysReg((32 + i) as u16), val);
            }
        }
        for i in 0u8..32 {
            let vreg = VRegIdx::new(i);
            let bytes = cpu.hart.regs.vpr().read_bytes(vreg);
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

/// Convert an EEW byte count (1/2/4/8) to the corresponding `MemWidth`.
const fn mem_width_from_eew_bytes(bytes: usize) -> crate::core::pipeline::signals::MemWidth {
    use crate::core::pipeline::signals::MemWidth as MW;
    match bytes {
        1 => MW::Byte,
        2 => MW::Half,
        8 => MW::Double,
        _ => MW::Word,
    }
}

impl ExecutionEngine for O3Engine {
    fn tick(
        &mut self,
        cpu: &mut SimState,
        rename_output: &mut Vec<RenameIssueEntry>,
        redirect_pending: &mut bool,
    ) {
        self.cycle += 1;
        self.mdp.tick();
        let now = self.cycle;

        // Squash recovery: ROB read ports are busy with reclaim / rename rebuild.
        if self.squash_stall_remaining > 0 {
            self.squash_stall_remaining -= 1;
            cpu.stats.stalls_squash += 1;
        }

        let pc_before_commit = cpu.hart.pc;

        let trap_event = commit::commit_stage(
            cpu,
            &mut self.common,
            &mut self.rob,
            &mut self.store_buffer,
            &mut self.scoreboard,
            &mut self.committed_rename_map,
            &mut self.free_list,
            self.width,
            Some(&mut self.load_queue),
            Some(&mut self.prf),
            Some(&mut self.checkpoints),
            Some(&mut self.vec_prf),
            Some(&mut self.vec_free_list),
            Some(&mut self.vec_store_buffer),
            redirect_pending,
        );

        if let Some((trap, pc)) = trap_event {
            // Full flush: committed_rename_map is used directly, no rebuild.
            let squashed = self.rob.len();
            self.flush(cpu);
            self.squash_stall_remaining = self.compute_squash_stall(squashed, 0);
            *redirect_pending = true;
            cpu.trap(&trap, pc);
            cpu.hart.committed_next_pc = cpu.hart.pc;
            return;
        }

        if cpu.hart.pc != pc_before_commit {
            let squashed = self.rob.len();
            self.flush(cpu);
            self.squash_stall_remaining = self.compute_squash_stall(squashed, 0);
            rename_output.clear();
            return;
        }

        // Intercept vec mem micro-ops before the normal writeback stage.
        {
            let mut scalar_wb = Vec::with_capacity(self.mem2_wb.len());
            let vec_entries = std::mem::take(&mut self.mem2_wb);
            for wb in vec_entries {
                if let Some(ref vme) = wb.vec_mem {
                    if wb.trap.is_none() && !vme.is_store {
                        let vlen_bits = self.vec_prf.vlen().bits();
                        let eew_bits = vme.eew.bytes() * 8;
                        let elems_per_reg = if eew_bits > 0 { vlen_bits / eew_bits } else { 1 };
                        let local = ElemIdx::new(vme.elem_idx.as_usize() % elems_per_reg);
                        self.vec_prf.write_element(vme.vd_phys, local, vme.eew, wb.load_data);
                    }
                    if !vme.is_store {
                        self.load_queue.deallocate_elem(wb.rob_tag, vme.elem_idx);
                    }
                    if let Some(parent) =
                        self.vec_mem_inflight.iter_mut().find(|m| m.rob_tag == wb.rob_tag)
                    {
                        parent.remaining = parent.remaining.saturating_sub(1);

                        // Fire chaining wakeup only on full completion: dependents bulk-read all elements.
                        if parent.remaining == 0 {
                            if !parent.wakeup_fired {
                                for j in 0..parent.vd_count as usize {
                                    self.vec_prf.mark_ready(parent.vd_phys[j]);
                                }
                                for j in 0..parent.vd_count as usize {
                                    self.issue_queue
                                        .wakeup_vec_phys(parent.vd_phys[j], &self.vec_prf);
                                }
                                parent.wakeup_fired = true;
                            }
                            self.rob.complete(parent.rob_tag, 0);
                        }
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

        writeback::writeback_stage(cpu, &mut self.mem2_wb, &mut self.rob);

        for (_tag, rd_phys, val) in &wb_wakeups {
            self.prf.write(*rd_phys, *val);
            self.issue_queue.wakeup_phys(*rd_phys, *val);
        }

        // Drain completed MSHRs: install lines in L1D and resume parked loads.
        if cpu.core.l1d_mshrs.capacity() > 0 {
            let completed = cpu.core.l1d_mshrs.drain_completions(now);
            for mshr_entry in completed {
                // miss latency already covers the write-back penalty.
                let (_penalty, evicted) = cpu.core.l1_d_cache.install_line_public_tracked(
                    mshr_entry.line_addr,
                    mshr_entry.is_write,
                    0,
                );

                if cpu.config.cache.inclusion_policy == crate::config::InclusionPolicy::Exclusive
                    && cpu.core.l2_cache.enabled
                    && let Some(ev) = evicted
                {
                    let _ = cpu.core.l2_cache.install_or_replace(ev.addr, ev.dirty, 0);
                    cpu.stats.exclusive_l1_to_l2_swaps += 1;
                }

                for waiter in mshr_entry.waiters {
                    if let Some(mut parked) = waiter.parked_entry {
                        parked.complete_cycle = now;
                        self.mem1_mem2.push(parked);
                    }
                }
            }
        }

        let wb_before = self.mem2_wb.len();
        let mem_violation = memory2::memory2_stage(
            cpu,
            &mut self.mem1_mem2,
            &mut self.mem2_wb,
            &mut self.store_buffer,
            Some(&mut self.load_queue),
            Some(&mut self.vec_store_buffer),
        );

        for entry in &self.mem2_wb[wb_before..] {
            if entry.ctrl.mem_write
                && let Some(store_tag) = self.mdp.store_resolved(entry.rob_tag)
            {
                self.issue_queue.wakeup_mem_dep(&[store_tag]);
            }
        }

        if let Some((violating_tag, store_pc)) = mem_violation {
            let violation_pc = self.rob.find_entry(violating_tag).map_or(cpu.hart.pc, |e| e.pc);

            self.mdp.violation(violation_pc, store_pc);

            // keep_tag must be a tag actually in the ROB; synthetic `tag-1` could be a use-after-free.
            let keep_tag = self.rob.prev_tag_of(violating_tag);

            cpu.stats.mem_ordering_violations += 1;
            cpu.stats.pipeline_flushes += 1;
            cpu.stats.stalls_control += 1;

            if let Some(keep_tag) = keep_tag {
                for entry in self.rob.iter_after(keep_tag) {
                    self.free_list.reclaim(entry.phys_dst);
                    for i in 0..entry.vec_dst_count as usize {
                        self.vec_free_list.reclaim(entry.vec_phys_dst[i]);
                    }
                }
                let squashed = self.rob.iter_after(keep_tag).count();
                cpu.stats.misprediction_penalty += squashed as u64;

                self.issue_queue.flush_after(keep_tag);
                self.rob.flush_after(keep_tag);
                self.store_buffer.flush_after(keep_tag);
                self.load_queue.flush_after(keep_tag);
                self.mdp.flush_after(keep_tag, &self.rob);
                cpu.core.l1d_mshrs.flush_after(keep_tag);

                self.mem1_mem2.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
                self.mem2_wb.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
                self.pending_results.retain(|p| p.entry.rob_tag.is_older_or_eq(keep_tag));
                self.vec_pending.retain(|v| v.rob_tag.is_older_or_eq(keep_tag));
                self.vec_mem_pending.retain(|m| m.entry.rob_tag.is_older_or_eq(keep_tag));
                self.vec_mem_inflight.retain(|m| m.rob_tag.is_older_or_eq(keep_tag));
                self.vec_store_buffer.flush_after(keep_tag);
                self.execute_mem1.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));

                // The violating load is not a branch, so checkpoint rebuild always applies.
                let surviving = self.rob.len();
                self.squash_stall_remaining = self.compute_squash_stall(squashed, surviving);
                cpu.stats.stalls_rename_rebuild += surviving.div_ceil(self.width.max(1)) as u64;
            } else {
                // Violating load is at ROB head (or older entry committed): full flush.
                for entry in self.rob.iter_all() {
                    self.free_list.reclaim(entry.phys_dst);
                    for i in 0..entry.vec_dst_count as usize {
                        self.vec_free_list.reclaim(entry.vec_phys_dst[i]);
                    }
                }
                let squashed = self.rob.len();
                cpu.stats.misprediction_penalty += squashed as u64;

                self.issue_queue.flush();
                self.rob.flush_all();
                self.store_buffer.flush_speculative();
                self.load_queue.flush();
                self.mdp.flush();
                cpu.core.l1d_mshrs.flush();

                self.mem1_mem2.clear();
                self.mem2_wb.clear();
                self.pending_results.clear();
                self.vec_pending.clear();
                self.vec_mem_pending.clear();
                self.vec_mem_inflight.clear();
                self.vec_store_buffer.flush_all();
                self.execute_mem1.clear();

                self.squash_stall_remaining = self.compute_squash_stall(squashed, 0);
            }

            self.rebuild_rename_map();
            self.scoreboard.rebuild_from_rob(&self.rob);
            if let Some(keep_tag) = keep_tag {
                self.checkpoints.flush_after(keep_tag);
            } else {
                self.checkpoints.flush_all();
            }

            cpu.hart.pc = violation_pc;
            *redirect_pending = true;
            rename_output.clear();
            return;
        }

        // Packet-based memory1 always accepts work and parks loads in
        // `common.outstanding_loads`. Backpressure comes from the L1D's
        // pending table when the cache is saturated, which surfaces as
        // mailbox-drain backlogs rather than a per-engine `mem1_busy` gate.
        let mut input = std::mem::take(&mut self.execute_mem1);
        memory1::memory1_stage(cpu, self, &mut input);
        self.execute_mem1.extend(input);
        let _ = now;

        // Backpressure only on undrained execute_mem1; pending_results drains after issue.
        let mem_backpressured = !self.execute_mem1.is_empty();

        if mem_backpressured {
            cpu.stats.stalls_backpressure += 1;
        }

        {
            let mut i = 0;
            while i < self.pending_results.len() {
                if self.pending_results[i].complete_cycle <= now {
                    let pr = self.pending_results.swap_remove(i);
                    let entry = pr.entry;
                    let fu_type = pr.fu_type;

                    cpu.stats.fu_utilization[fu_type as usize] += 1;

                    if entry.ctrl.mem_read
                        || entry.ctrl.mem_write
                        || entry.ctrl.atomic_op != crate::core::pipeline::signals::AtomicOp::None
                    {
                        self.execute_mem1.push(entry);
                    } else if !pr.speculative_written {
                        // Non-pipelined non-mem (IntDiv, FpDivSqrt, system): write PRF + wakeup now.
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
                    } else {
                        // Pipelined non-mem already retired at issue; commit retires from ROB.
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

        // Set when any issued instruction returns needs_flush=true.
        let mut flush_keep_tag: Option<crate::core::pipeline::rob::RobTag> = None;

        {
            let issued = self.issue_queue.select(
                self.width,
                &self.store_buffer,
                &self.rob,
                self.load_ports,
                self.store_ports,
                Some(&self.prf),
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
                        cpu,
                        Some(&self.prf),
                        Some(&self.vec_prf),
                        mem_dep,
                    );
                    debug_assert!(ok, "re-dispatch after mem backpressure failed");
                    continue;
                }

                // Structural hazard: stall vec stores back to IQ if the VSB is full.
                if fu_type == FuType::VecMem
                    && is_vec_store(entry.ctrl.vec_op)
                    && self.vec_store_buffer.free_slots() == 0
                {
                    cpu.stats.stalls_fu_structural += 1;
                    let ok = self.issue_queue.dispatch(
                        entry,
                        &self.rob,
                        cpu,
                        Some(&self.prf),
                        Some(&self.vec_prf),
                        mem_dep,
                    );
                    debug_assert!(ok, "re-dispatch after VSB-full failed");
                    continue;
                }

                if !self.fu_pool.has_free(fu_type, now) {
                    cpu.stats.stalls_fu_structural += 1;
                    stalled_fu = true;
                    let ok = self.issue_queue.dispatch(
                        entry,
                        &self.rob,
                        cpu,
                        Some(&self.prf),
                        Some(&self.vec_prf),
                        mem_dep,
                    );
                    debug_assert!(ok, "re-dispatch after FU stall failed");
                    continue;
                }

                if is_mem_instr {
                    self.mdp.issued(rob_tag);
                }

                // vsetvl* run synchronously in execute_one; exclude from deferred VecPrfView.
                let is_vec_non_mem = fu_type.is_vector()
                    && fu_type != FuType::VecMem
                    && !matches!(
                        entry.ctrl.vec_op,
                        VectorOp::Vsetvli | VectorOp::Vsetivli | VectorOp::Vsetvl
                    );
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

                let saved_entry =
                    if is_vec_non_mem || is_vec_mem_op { Some(entry.clone()) } else { None };

                let (complete_cycle, is_pipelined) = if is_vec_non_mem {
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
                    let cc = self.fu_pool.acquire_with_latency(fu_type, now, latency);
                    (cc, pipelined)
                } else if is_vec_mem_op {
                    // VecMem FU is 1 cycle (address gen); micro-op latency comes from Memory1/2.
                    let cc = self.fu_pool.acquire(fu_type, now);
                    (cc, true)
                } else {
                    let cc = self.fu_pool.acquire(fu_type, now);
                    let p = self.fu_pool.is_pipelined(fu_type);
                    (cc, p)
                };

                let (ex_result, flush) =
                    execute::execute_one(cpu, entry, &mut self.rob, redirect_pending);
                issued_count += 1;

                if is_vec_non_mem
                    && ex_result.trap.is_none()
                    && let Some(saved) = saved_entry.as_ref()
                {
                    use crate::core::units::vpu::execute::execute_vec_op_on;
                    use crate::core::units::vpu::lane_model;

                    // Build arch→phys mapping from rename-time physregs so later renames don't alias.
                    let mut mapping = [VecPhysReg::ZERO; 32];
                    for i in 0..32u8 {
                        mapping[i as usize] = self.rename_map.get_vec(VRegIdx::new(i));
                    }

                    {
                        let base = saved.ctrl.vs2.as_u8() as usize;
                        for i in 0..saved.vec_src2_count as usize {
                            if base + i < 32 {
                                mapping[base + i] = saved.vs2_phys[i];
                            }
                        }
                    }
                    {
                        let base = saved.ctrl.vs1.as_u8() as usize;
                        for i in 0..saved.vec_src1_count as usize {
                            if base + i < 32 {
                                mapping[base + i] = saved.vs1_phys[i];
                            }
                        }
                    }
                    if let Some((vd_phys_arr, vd_cnt, vd_reg)) = vec_dst_info {
                        let base = vd_reg.as_u8() as usize;
                        for i in 0..vd_cnt as usize {
                            if base + i < 32 {
                                // Pre-copy old vd so tail/mask-undisturbed reads see correct baseline.
                                if i < saved.vec_src3_count as usize {
                                    self.vec_prf.copy_reg(vd_phys_arr[i], saved.vs3_phys[i]);
                                }
                                mapping[base + i] = vd_phys_arr[i];
                            }
                        }
                    }
                    if !saved.mask_phys.is_zero() {
                        mapping[0] = saved.mask_phys;
                    }

                    // Use dispatch-time CSR snapshot so in-flight vsetvl can't corrupt vtype/vl.
                    let vec_result_or_trap = {
                        let mut view = VecPrfView::new(&mut self.vec_prf, mapping);
                        execute_vec_op_on(
                            &mut view,
                            saved.vec_vtype,
                            saved.vec_vl,
                            saved.vec_vstart,
                            saved.vec_vxrm,
                            saved.vec_frm,
                            cpu.config.isa.vector.elen,
                            cpu.config.isa.vector.zvfh,
                            saved,
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
                            speculative_written: false,
                        });
                    }

                    let keep_tag = ex_result.rob_tag;
                    if flush {
                        flush_keep_tag = Some(keep_tag);
                        break;
                    }
                    continue;
                }

                if is_vec_mem_op
                    && ex_result.trap.is_none()
                    && let Some(saved) = saved_entry.as_ref()
                {
                    let vec_op = ex_result.ctrl.vec_op;
                    let is_store = is_vec_store(vec_op);
                    let vd_count = vec_dst_info.map_or(0u8, |(_, c, _)| c);
                    let vd_phys_arr = vec_dst_info.map_or([VecPhysReg::ZERO; 8], |(p, _, _)| p);

                    // Reject illegal EMUL (>8) before generate_element_addrs_vrf would panic.
                    let vtype = crate::core::units::vpu::types::parse_vtype(saved.vec_vtype);
                    if let Err(trap) = crate::core::units::vpu::mem::check_vec_mem_emul(
                        ex_result.inst,
                        vec_op,
                        &saved.ctrl,
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
                        let base = saved.ctrl.vs2.as_u8() as usize;
                        for i in 0..saved.vec_src2_count as usize {
                            if base + i < 32 {
                                mapping[base + i] = saved.vs2_phys[i];
                            }
                        }
                    }
                    {
                        let base = saved.ctrl.vd.as_u8() as usize;
                        for i in 0..saved.vec_src3_count as usize {
                            if base + i < 32 {
                                mapping[base + i] = saved.vs3_phys[i];
                            }
                        }
                    }
                    if !saved.mask_phys.is_zero() {
                        mapping[0] = saved.mask_phys;
                    }
                    let micro_ops = {
                        let view = VecPrfView::new(&mut self.vec_prf, mapping);
                        generate_element_addrs_vrf(
                            &view,
                            ex_result.alu,
                            ex_result.store_data as i64,
                            &saved.ctrl,
                            saved.vec_vtype,
                            saved.vec_vl as usize,
                            saved.vec_vstart as usize,
                            vec_op,
                            &vd_phys_arr,
                            vd_count,
                        )
                    };

                    // Pre-copy old vd so tail / mask-undisturbed elements observe prior values.
                    if !is_store && let Some((vd_phys_arr_pre, vd_cnt_pre, _)) = vec_dst_info {
                        let copy_count = (vd_cnt_pre as usize).min(saved.vec_src3_count as usize);
                        for (i, &dst) in vd_phys_arr_pre.iter().enumerate().take(copy_count) {
                            self.vec_prf.copy_reg(dst, saved.vs3_phys[i]);
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
                        let mut all_micro_ops: std::collections::VecDeque<VecMemMicroOp> =
                            std::collections::VecDeque::with_capacity(total);
                        for mop in micro_ops {
                            let eew_width = mem_width_from_eew_bytes(mop.eew.bytes());
                            let mut ctrl = ex_result.ctrl;
                            ctrl.mem_read = !is_store;
                            ctrl.mem_write = is_store;
                            ctrl.width = eew_width;

                            let vec_elem = VecMemElement {
                                elem_idx: mop.elem_idx,
                                eew: mop.eew,
                                vd_phys: mop.vd_phys,
                                is_store,
                            };
                            all_micro_ops.push_back(VecMemMicroOp {
                                entry: ExMem1Entry {
                                    rob_tag: ex_result.rob_tag,
                                    pc: ex_result.pc,
                                    inst: ex_result.inst,
                                    inst_size: ex_result.inst_size,
                                    rd: ex_result.rd,
                                    rd_phys: ex_result.rd_phys,
                                    alu: mop.vaddr.val(),
                                    store_data: mop.store_data,
                                    ctrl,
                                    trap: None,
                                    exception_stage: None,
                                    fp_flags: 0,
                                    sfence_vma: None,
                                    vec_mem: Some(vec_elem),
                                },
                                elem_idx: mop.elem_idx,
                                eew: mop.eew,
                                vd_phys: mop.vd_phys,
                                is_store,
                            });
                        }

                        if is_store {
                            let ok = self.vec_store_buffer.allocate(ex_result.rob_tag, total);
                            debug_assert!(ok, "VSB allocate failed despite pre-check");
                        }

                        self.vec_mem_inflight.push(VecMemInflight {
                            rob_tag: ex_result.rob_tag,
                            remaining: total,
                            vd_phys: vd_phys_arr,
                            vd_count,
                            wakeup_fired: false,
                            pending_micro_ops: all_micro_ops,
                        });
                    }

                    let keep_tag = ex_result.rob_tag;
                    if flush {
                        flush_keep_tag = Some(keep_tag);
                        break;
                    }
                    continue;
                }

                let is_mem = ex_result.ctrl.mem_read
                    || ex_result.ctrl.mem_write
                    || ex_result.ctrl.atomic_op != crate::core::pipeline::signals::AtomicOp::None;

                // Pipelined non-mem: wake dependents immediately so they can issue next cycle.
                let speculative_written = if !is_mem && is_pipelined && ex_result.trap.is_none() {
                    let val = if ex_result.ctrl.control_flow == ControlFlow::Jump {
                        ex_result.pc.wrapping_add(ex_result.inst_size.as_u64())
                    } else {
                        ex_result.alu
                    };
                    if ex_result.fp_flags != 0 {
                        self.rob.set_fp_flags(ex_result.rob_tag, ex_result.fp_flags);
                    }
                    if let Some(info) = ex_result.sfence_vma {
                        self.rob.set_sfence_vma(ex_result.rob_tag, info);
                    }
                    self.rob.complete(ex_result.rob_tag, val);
                    self.prf.write(ex_result.rd_phys, val);
                    self.issue_queue.wakeup_phys(ex_result.rd_phys, val);
                    true
                } else {
                    false
                };

                // Speculative load wakeup assuming L1D hit (only if MSHRs are configured).
                let is_load = ex_result.ctrl.mem_read && !ex_result.ctrl.mem_write;
                if is_load && ex_result.trap.is_none() && cpu.core.l1d_mshrs.capacity() > 0 {
                    self.issue_queue.speculative_wakeup_phys(ex_result.rd_phys);
                }

                let keep_tag = ex_result.rob_tag;
                self.pending_results.push(PendingResult {
                    entry: ex_result,
                    complete_cycle,
                    fu_type,
                    speculative_written,
                });

                if flush {
                    flush_keep_tag = Some(keep_tag);
                    break;
                }
            }

            if issued_count == 0 && !stalled_fu && !self.issue_queue.is_empty() {
                cpu.stats.stalls_data += 1;
            }
        }

        if let Some(keep_tag) = flush_keep_tag {
            cpu.stats.stalls_control += 1;
            cpu.stats.pipeline_flushes += 1;

            if let Some(entry) = self.rob.find_entry(keep_tag) {
                if matches!(entry.ctrl.control_flow, ControlFlow::Branch | ControlFlow::Jump) {
                    cpu.stats.flushes_branch += 1;
                } else {
                    cpu.stats.flushes_system += 1;
                }
            } else {
                cpu.stats.flushes_system += 1;
            }

            rename_output.clear();

            // keep_tag may have been committed in step 1; if so, flush everything younger.
            let keep_in_rob = self.rob.find_entry(keep_tag).is_some();

            let squashed: usize;
            if keep_in_rob {
                squashed = self.rob.iter_after(keep_tag).count();
                cpu.stats.misprediction_penalty += squashed as u64;
                for entry in self.rob.iter_after(keep_tag) {
                    self.free_list.reclaim(entry.phys_dst);
                    for i in 0..entry.vec_dst_count as usize {
                        self.vec_free_list.reclaim(entry.vec_phys_dst[i]);
                    }
                }
                // flush_after, not flush: older un-issued IQ entries must survive or deadlock the pipeline.
                self.issue_queue.flush_after(keep_tag);
                self.rob.flush_after(keep_tag);
                self.store_buffer.flush_after(keep_tag);
                self.load_queue.flush_after(keep_tag);
                self.mdp.flush_after(keep_tag, &self.rob);
                cpu.core.l1d_mshrs.flush_after(keep_tag);
            } else {
                // keep_tag already committed: flush everything in-flight.
                for entry in self.rob.iter_all() {
                    self.free_list.reclaim(entry.phys_dst);
                    for i in 0..entry.vec_dst_count as usize {
                        self.vec_free_list.reclaim(entry.vec_phys_dst[i]);
                    }
                }
                squashed = self.rob.len();
                cpu.stats.misprediction_penalty += squashed as u64;
                self.issue_queue.flush();
                self.rob.flush_all();
                self.store_buffer.flush_speculative();
                self.load_queue.flush();
                self.mdp.flush();
                cpu.core.l1d_mshrs.flush();
            }
            self.mem1_mem2.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
            self.mem2_wb.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
            self.pending_results.retain(|p| p.entry.rob_tag.is_older_or_eq(keep_tag));
            self.vec_pending.retain(|v| v.rob_tag.is_older_or_eq(keep_tag));
            self.vec_mem_pending.retain(|m| m.entry.rob_tag.is_older_or_eq(keep_tag));
            self.vec_mem_inflight.retain(|m| m.rob_tag.is_older_or_eq(keep_tag));
            self.vec_store_buffer.flush_after(keep_tag);
            self.execute_mem1.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
            // Restore speculative rename map: checkpoint (O(1)) or forward ROB walk rebuild.
            let surviving = self.rob.len();
            if self.checkpoints.capacity() > 0 {
                if let Some(ckpt) = self.checkpoints.find_by_tag(keep_tag) {
                    self.rename_map = ckpt.rename_map.clone();
                    cpu.hart.csrs.vtype = ckpt.vtype;
                    cpu.hart.csrs.vl = ckpt.vl;
                    cpu.hart.csrs.frm = ckpt.frm;
                    cpu.hart.csrs.vxrm = ckpt.vxrm;
                    cpu.hart.csrs.vstart = ckpt.vstart;
                    self.squash_stall_remaining = self.compute_squash_stall(squashed, 0);
                } else {
                    self.rebuild_rename_map();
                    self.squash_stall_remaining = self.compute_squash_stall(squashed, surviving);
                    cpu.stats.stalls_rename_rebuild += surviving.div_ceil(self.width.max(1)) as u64;
                }
                self.checkpoints.flush_after(keep_tag);
            } else {
                self.rebuild_rename_map();
                self.squash_stall_remaining = self.compute_squash_stall(squashed, surviving);
                cpu.stats.stalls_rename_rebuild += surviving.div_ceil(self.width.max(1)) as u64;
            }
            self.scoreboard.rebuild_from_rob(&self.rob);
        }

        if flush_keep_tag.is_none() {
            let entries = std::mem::take(rename_output);
            for entry in entries {
                let is_load = entry.ctrl.mem_read;
                let is_store = entry.ctrl.mem_write;
                let is_atomic = entry.ctrl.atomic_op
                    != crate::core::pipeline::signals::AtomicOp::None;
                let mem_dep = self
                    .mdp
                    .dispatch(entry.pc, entry.rob_tag, is_load, is_store, is_atomic);
                let ok = self.issue_queue.dispatch(
                    entry,
                    &self.rob,
                    cpu,
                    Some(&self.prf),
                    Some(&self.vec_prf),
                    mem_dep,
                );
                debug_assert!(ok, "IQ dispatch failed — rename budget should prevent this");
            }
        }

        let mdp_stats = self.mdp.stats();
        cpu.stats.mdp_predictions_bypass = mdp_stats.predictions_bypass;
        cpu.stats.mdp_predictions_wait_all = mdp_stats.predictions_wait_all;
        cpu.stats.mdp_predictions_wait_for = mdp_stats.predictions_wait_for;
        cpu.stats.mdp_violations = mdp_stats.violations;
    }

    fn can_accept(&self) -> usize {
        // Squash recovery monopolises ROB read ports; rename can't dispatch during it.
        if self.squash_stall_remaining > 0 {
            return 0;
        }
        let rob_free = self.rob.free_slots();
        let sb_free = self.store_buffer.free_slots();
        let lq_free = self.load_queue.free_slots();
        let iq_free = self.issue_queue.available_slots();
        let prf_free = self.free_list.available();
        let vec_prf_free = self.vec_free_list.available();
        rob_free
            .min(sb_free)
            .min(lq_free)
            .min(iq_free)
            .min(prf_free)
            .min(vec_prf_free)
            .min(self.width)
    }

    fn flush(&mut self, cpu: &mut SimState) {
        // Drain committed VSB writes; trap-driven flushes still owe pre-trap retired stores.
        self.vec_store_buffer.drain_all_committed(cpu, &mut self.common);

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
        self.scoreboard.flush();
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
        self.mem1_mem2.clear();
        self.mem2_wb.clear();
        cpu.core.l1d_mshrs.flush();
        cpu.core.branch_predictor.repair_to_committed();

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

    fn read_csr_speculative(&self, cpu: &crate::sim::SimState, addr: crate::common::CsrAddr) -> u64 {
        cpu.csr_read(addr)
    }

    fn rob(&self) -> &Rob {
        &self.rob
    }

    fn rob_mut(&mut self) -> &mut Rob {
        &mut self.rob
    }

    fn store_buffer(&self) -> &StoreBuffer {
        &self.store_buffer
    }

    fn store_buffer_mut(&mut self) -> &mut StoreBuffer {
        &mut self.store_buffer
    }

    fn scoreboard(&self) -> &Scoreboard {
        &self.scoreboard
    }

    fn scoreboard_mut(&mut self) -> &mut Scoreboard {
        &mut self.scoreboard
    }

    fn execute_mem1_mut(&mut self) -> &mut Vec<crate::core::pipeline::latches::ExMem1Entry> {
        &mut self.execute_mem1
    }

    fn mem1_mem2_mut(
        &mut self,
    ) -> &mut Vec<crate::core::pipeline::latches::Mem1Mem2Entry> {
        &mut self.mem1_mem2
    }

    fn common(&self) -> &crate::core::pipeline::engine::BackendCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut crate::core::pipeline::engine::BackendCommon {
        &mut self.common
    }

    fn rename_map(&self) -> &RenameMap {
        &self.rename_map
    }

    fn rename_map_mut(&mut self) -> &mut RenameMap {
        &mut self.rename_map
    }

    fn prf(&self) -> &PhysRegFile {
        &self.prf
    }

    fn prf_mut(&mut self) -> &mut PhysRegFile {
        &mut self.prf
    }

    fn free_list_mut(&mut self) -> &mut FreeList<PhysReg> {
        &mut self.free_list
    }

    fn load_queue_mut(&mut self) -> Option<&mut LoadQueue> {
        Some(&mut self.load_queue)
    }

    fn has_prf(&self) -> bool {
        true
    }

    fn has_register_renaming(&self) -> bool {
        true
    }

    fn checkpoint_table(&self) -> &CheckpointTable {
        &self.checkpoints
    }

    fn checkpoint_table_mut(&mut self) -> &mut CheckpointTable {
        &mut self.checkpoints
    }

    fn checkpoint_count(&self) -> usize {
        self.checkpoints.capacity()
    }

    fn vec_prf(&self) -> &VecPhysRegFile {
        &self.vec_prf
    }

    fn vec_prf_mut(&mut self) -> &mut VecPhysRegFile {
        &mut self.vec_prf
    }

    fn vec_free_list_mut(&mut self) -> &mut FreeList<VecPhysReg> {
        &mut self.vec_free_list
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
        let mut cpu = SimState::build(&config, "");

        let mut engine = O3Engine::new(&config, crate::sim::components::PipelineId::new(0), crate::sim::components::CacheId::new(0), crate::sim::components::CacheId::new(1));
        assert_eq!(engine.width, config.pipeline.width);

        engine.flush(&mut cpu);
        assert_eq!(engine.execute_mem1.len(), 0);
    }

    #[test]
    fn test_o3_engine_sync_arch_regs() {
        let config = Config::default();
        let mut cpu = SimState::build(&config, "");
        let mut engine = O3Engine::new(&config, crate::sim::components::PipelineId::new(0), crate::sim::components::CacheId::new(0), crate::sim::components::CacheId::new(1));

        cpu.hart.regs.write(RegIdx::new(1), 42);
        engine.sync_arch_regs(&cpu);

        assert_eq!(engine.prf.read(crate::core::pipeline::prf::PhysReg(1)), 42);
    }
}
