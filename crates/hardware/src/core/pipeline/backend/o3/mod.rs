//! Out-of-Order (O3) backend: issue queue with wakeup/select, out-of-order execution.
//!
//! The O3 backend reuses shared pipeline stages (Memory1, Memory2, Writeback,
//! Commit) and shared hardware units (ALU, FPU, BRU), but has its own:
//! - **IssueQueue**: CAM-style with wakeup/select (vs FIFO for in-order)
//! - **execute_one()**: single-instruction execute (vs batch for in-order)

pub mod execute;
pub mod fu_pool;
pub mod issue_queue;

use crate::config::Config;
use crate::core::Cpu;
use crate::core::pipeline::backend::shared::{commit, memory1, memory2, writeback};
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::free_list::FreeList;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry, Mem2WbEntry, RenameIssueEntry};
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::prf::PhysRegFile;
use crate::core::pipeline::rename_map::RenameMap;
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::store_buffer::StoreBuffer;

use self::fu_pool::{FuPool, FuType};
use self::issue_queue::IssueQueue;

/// A result that has been computed but not yet written back (pending due to latency).
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
    pub free_list: FreeList,
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
    /// Execute -> Memory1 latch.
    pub execute_mem1: Vec<ExMem1Entry>,
    /// Memory1 -> Memory2 latch.
    pub mem1_mem2: Vec<Mem1Mem2Entry>,
    /// Memory2 -> Writeback latch.
    pub mem2_wb: Vec<Mem2WbEntry>,
    /// Current simulation cycle (for FU latency tracking).
    pub cycle: u64,
}

impl O3Engine {
    /// Creates a new O3 engine from config.
    pub fn new(config: &Config) -> Self {
        let rob_size = config.pipeline.rob_size;
        let prf_gpr_size = config.pipeline.prf_gpr_size;
        let prf_fpr_size = config.pipeline.prf_fpr_size;
        let prf_total = prf_gpr_size + prf_fpr_size;
        // GPR: arch regs 0..32 occupy slots 0..32; FPR: arch regs 0..32 occupy slots 32..64.
        // Free list starts with slots 64..prf_total available.
        let num_arch = 64; // 32 GPR + 32 FPR

        // Initialize PRF: identity-mapped slots (0..num_arch) are all ready with value 0.
        // PhysReg(0) is hardwired zero (always ready). Regs 1..num_arch represent the
        // initial architectural register state — all readable as 0 at startup.
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
            execute_mem1: Vec::with_capacity(config.pipeline.width),
            mem1_mem2: Vec::with_capacity(config.pipeline.width),
            mem2_wb: Vec::with_capacity(config.pipeline.width),
            cycle: 0,
        }
    }

    /// Copy initial architectural register values into the identity-mapped PRF slots.
    ///
    /// Must be called after the CPU's architectural register file has been initialized
    /// (e.g., with the stack pointer) but before the first pipeline tick. The rename
    /// map identity-maps arch reg `i` → PhysReg(i) for GPRs and arch reg `i` →
    /// PhysReg(32 + i) for FPRs, so we write the CPU's register values into those slots.
    pub fn sync_arch_regs(&mut self, cpu: &crate::core::Cpu) {
        use crate::core::pipeline::prf::PhysReg;
        // GPRs: arch reg i → PhysReg(i), skip x0 (hardwired zero)
        for i in 1..32 {
            let val = cpu.regs.read(i);
            if val != 0 {
                self.prf.write(PhysReg(i as u16), val);
            }
        }
        // FPRs: arch reg i → PhysReg(32 + i)
        for i in 0..32 {
            let val = cpu.regs.read_f(i);
            if val != 0 {
                self.prf.write(PhysReg((32 + i) as u16), val);
            }
        }
    }

    /// Rebuild the speculative rename map after a partial flush (misprediction).
    ///
    /// Starts from the committed map and re-applies surviving ROB entries in order.
    fn rebuild_rename_map(&mut self) {
        self.rename_map = self.committed_rename_map.clone();
        // Walk surviving ROB entries in program order (head → tail) and re-apply
        // each entry's phys_dst mapping, restoring the speculative state.
        for entry in self.rob.iter_in_order() {
            if entry.ctrl.reg_write && entry.rd != 0 {
                self.rename_map.set(entry.rd, false, entry.phys_dst);
            } else if entry.ctrl.fp_reg_write {
                self.rename_map.set(entry.rd, true, entry.phys_dst);
            }
        }
    }
}

impl ExecutionEngine for O3Engine {
    fn tick(&mut self, cpu: &mut Cpu, rename_output: &mut Vec<RenameIssueEntry>) {
        // Backend stages run in reverse order (drain from commit to issue)
        self.cycle += 1;
        let now = self.cycle;

        let pc_before_commit = cpu.pc;

        // ── 1. Commit ──────────────────────────────────────────────────
        let trap_event = commit::commit_stage(
            cpu,
            &mut self.rob,
            &mut self.store_buffer,
            &mut self.scoreboard,
            &mut self.committed_rename_map,
            &mut self.free_list,
            self.width,
            Some(&mut self.load_queue),
        );

        // Handle trap: flush everything
        if let Some((trap, pc)) = trap_event {
            if cpu.trace {
                eprintln!("BE  * HANDLING TRAP: {:?} at PC {:#x}", trap, pc);
            }
            self.flush(cpu);
            cpu.redirect_pending = true;
            cpu.trap(trap, pc);
            return;
        }

        // Handle MRET/SRET redirect
        if cpu.pc != pc_before_commit {
            if cpu.trace {
                eprintln!(
                    "BE  * MRET/SRET REDIRECT: {:#x} -> {:#x}, flushing backend",
                    pc_before_commit, cpu.pc
                );
            }
            self.flush(cpu);
            rename_output.clear();
            return;
        }

        // ── 2. Writeback + Wakeup ──────────────────────────────────────
        // Peek at mem2_wb to know which entries will complete, so we can
        // wakeup dependents in the issue queue via PRF.
        let wb_wakeups: Vec<_> = self
            .mem2_wb
            .iter()
            .filter(|wb| wb.trap.is_none())
            .map(|wb| {
                let val = if wb.ctrl.mem_read {
                    wb.load_data
                } else if wb.ctrl.jump {
                    wb.pc.wrapping_add(wb.inst_size)
                } else {
                    wb.alu
                };
                (wb.rob_tag, wb.rd_phys, val)
            })
            .collect();

        writeback::writeback_stage(cpu, &mut self.mem2_wb, &mut self.rob);

        // Broadcast wakeups to PRF + issue queue
        for (_tag, rd_phys, val) in &wb_wakeups {
            self.prf.write(*rd_phys, *val);
            self.issue_queue.wakeup_phys(*rd_phys, *val);
        }

        // ── 3. Memory2 ────────────────────────────────────────────────
        let mem_violation = memory2::memory2_stage(
            cpu,
            &mut self.mem1_mem2,
            &mut self.mem2_wb,
            &mut self.store_buffer,
            &mut self.rob,
            Some(&mut self.load_queue),
        );

        // Handle memory ordering violation: a store resolved its address and
        // overlapped with a younger load that already executed with stale data.
        // Flush from the violating load onward and redirect to re-fetch it.
        if let Some(violating_tag) = mem_violation {
            if cpu.trace {
                eprintln!(
                    "BE  * MEMORY ORDERING VIOLATION: load rob_tag={}, flushing",
                    violating_tag.0
                );
            }
            // Find the violating load's PC from the ROB
            let violation_pc = self
                .rob
                .find_entry(violating_tag)
                .map(|e| e.pc)
                .unwrap_or(cpu.pc);

            // keep_tag = tag before the violating load (everything older is kept)
            let keep_tag = crate::core::pipeline::rob::RobTag(violating_tag.0.saturating_sub(1));

            // Reclaim physical registers for squashed entries
            for entry in self.rob.iter_after(keep_tag) {
                self.free_list.reclaim(entry.phys_dst);
            }
            let squashed = self.rob.iter_after(keep_tag).count() as u64;
            cpu.stats.misprediction_penalty += squashed;
            cpu.stats.mem_ordering_violations += 1;
            cpu.stats.pipeline_flushes += 1;
            cpu.stats.stalls_control += 1;

            self.issue_queue.flush_after(keep_tag);
            self.rob.flush_after(keep_tag);
            self.store_buffer.flush_after(keep_tag);
            self.load_queue.flush_after(keep_tag);

            self.mem1_mem2.retain(|e| e.rob_tag.0 <= keep_tag.0);
            self.mem2_wb.retain(|e| e.rob_tag.0 <= keep_tag.0);
            self.pending_results
                .retain(|p| p.entry.rob_tag.0 <= keep_tag.0);
            self.execute_mem1.retain(|e| e.rob_tag.0 <= keep_tag.0);

            self.rebuild_rename_map();
            self.scoreboard.rebuild_from_rob(&self.rob);

            cpu.pc = violation_pc;
            cpu.redirect_pending = true;
            rename_output.clear();
            return;
        }

        // ── 4. Memory1 ────────────────────────────────────────────────
        // Per-operation latency: memory1 is gated by the max complete_cycle
        // of recently produced entries. Two independent loads stall for
        // max(latency) instead of sum(latency).
        //
        // Check if the memory1 pipeline is still busy (any entry from a
        // previous memory1 call hasn't completed yet).
        let mem1_busy = self.mem1_mem2.iter().any(|e| {
            let is_mem = e.ctrl.mem_read || e.ctrl.mem_write;
            is_mem && e.complete_cycle > now
        });

        if mem1_busy {
            cpu.stats.stalls_mem += 1;
        } else {
            memory1::memory1_stage(
                cpu,
                &mut self.execute_mem1,
                &mut self.mem1_mem2,
                now,
                Some(&mut self.load_queue),
            );
        }

        // ── 5. Backpressure check ──────────────────────────────────────
        // Backpressured if execute_mem1 is not empty (memory pipeline in use)
        // OR if there are pending memory results completing this cycle that will
        // be pushed into execute_mem1 in step 6a.
        let has_pending_mem = self.pending_results.iter().any(|p| {
            let is_mem = p.entry.ctrl.mem_read
                || p.entry.ctrl.mem_write
                || p.entry.ctrl.atomic_op != crate::core::pipeline::signals::AtomicOp::None;
            is_mem && p.complete_cycle <= now + 1
        });
        let backpressured = !self.execute_mem1.is_empty() || has_pending_mem;

        if backpressured {
            cpu.stats.stalls_backpressure += 1;
        }

        if cpu.trace && backpressured {
            eprintln!(
                "BE  backpressure={} ex_mem1={} iq={}",
                backpressured,
                self.execute_mem1.len(),
                self.issue_queue.available_slots()
            );
        }

        // ── 6a. Drain completed pending results ────────────────────────
        {
            let mut i = 0;
            while i < self.pending_results.len() {
                if self.pending_results[i].complete_cycle <= now {
                    let pr = self.pending_results.swap_remove(i);
                    let entry = pr.entry;
                    let fu_type = pr.fu_type;

                    // Update FU utilization stat
                    cpu.stats.fu_utilization[fu_type as usize] += 1;

                    if entry.ctrl.mem_read
                        || entry.ctrl.mem_write
                        || entry.ctrl.atomic_op != crate::core::pipeline::signals::AtomicOp::None
                    {
                        // Memory ops: push to memory pipeline
                        self.execute_mem1.push(entry);
                    } else if !pr.speculative_written {
                        // Non-memory, non-pipelined (e.g. IntDiv, FpDivSqrt, system): write PRF + wakeup now
                        let val = if entry.ctrl.jump {
                            entry.pc.wrapping_add(entry.inst_size)
                        } else {
                            entry.alu
                        };
                        if entry.fp_flags != 0 {
                            use crate::core::arch::csr;
                            cpu.csrs.fflags |= entry.fp_flags as u64;
                            cpu.csrs.mstatus =
                                (cpu.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                            cpu.csrs.sstatus =
                                (cpu.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                        }
                        // For CSR instructions: apply the deferred CSR write NOW (at complete
                        // time) rather than waiting for commit. This prevents a race where
                        // younger FP instructions execute speculatively and set fflags before
                        // the CSR write (e.g. fsflags fflags=0) has been applied. Since CSR
                        // instructions are serializing (they flush all younger instructions
                        // before completing), applying the write here is safe and correct.
                        if let Some(csr_entry) = self.rob.find_entry(entry.rob_tag)
                            && let Some(ref csr_update) = csr_entry.csr_update.clone()
                            && !csr_update.applied
                        {
                            use crate::core::arch::csr as csr_mod;
                            if csr_update.addr != csr_mod::SATP {
                                // SATP drain is handled at commit; skip eager apply.
                                cpu.csr_write(csr_update.addr, csr_update.new_val);
                                self.rob.mark_csr_applied(entry.rob_tag);
                            }
                        }
                        self.rob.complete(entry.rob_tag, val);
                        self.prf.write(entry.rd_phys, val);
                        self.issue_queue.wakeup_phys(entry.rd_phys, val);
                        // Still push through the memory pipeline for writeback
                        self.execute_mem1.push(entry);
                    } else {
                        // Pipelined non-memory: speculative wakeup already done at issue
                        // Just push through to writeback
                        self.execute_mem1.push(entry);
                    }
                } else {
                    i += 1;
                }
            }
        }

        // ── 6. Issue + Execute ─────────────────────────────────────────
        // `flush_keep_tag`: the rob_tag of the instruction that triggered a
        // flush (branch misprediction, CSR, FENCE.I, etc.).  Set when any
        // issued instruction returns needs_flush=true.
        let mut flush_keep_tag: Option<crate::core::pipeline::rob::RobTag> = None;

        if !backpressured {
            let issued = self
                .issue_queue
                .select(self.width, &self.store_buffer, &self.rob);

            let mut issued_count = 0;
            let mut stalled_fu = false;

            for entry in issued {
                let fu_type = FuType::classify(&entry.ctrl);

                // Check for structural hazard (no free FU of required type)
                if !self.fu_pool.has_free(fu_type, now) {
                    cpu.stats.stalls_fu_structural += 1;
                    stalled_fu = true;
                    // Leave entry in IQ (select already removed it — re-dispatch needed)
                    // For simplicity: re-dispatch back into IQ
                    let ok = self
                        .issue_queue
                        .dispatch(entry, &self.rob, cpu, Some(&self.prf));
                    debug_assert!(ok, "re-dispatch after FU stall failed");
                    continue;
                }

                let complete_cycle = self.fu_pool.acquire(fu_type, now);
                let is_pipelined = self.fu_pool.is_pipelined(fu_type);

                let (ex_result, flush) = execute::execute_one(cpu, entry, &mut self.rob);
                issued_count += 1;

                let is_mem = ex_result.ctrl.mem_read
                    || ex_result.ctrl.mem_write
                    || ex_result.ctrl.atomic_op != crate::core::pipeline::signals::AtomicOp::None;

                // For pipelined non-memory instructions: speculative wakeup immediately
                // so dependent instructions can be selected on the very next cycle.
                let speculative_written = if !is_mem && is_pipelined && ex_result.trap.is_none() {
                    let val = if ex_result.ctrl.jump {
                        ex_result.pc.wrapping_add(ex_result.inst_size)
                    } else {
                        ex_result.alu
                    };
                    if ex_result.fp_flags != 0 {
                        use crate::core::arch::csr;
                        cpu.csrs.fflags |= ex_result.fp_flags as u64;
                        cpu.csrs.mstatus =
                            (cpu.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                        cpu.csrs.sstatus =
                            (cpu.csrs.sstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
                    }
                    self.rob.complete(ex_result.rob_tag, val);
                    self.prf.write(ex_result.rd_phys, val);
                    self.issue_queue.wakeup_phys(ex_result.rd_phys, val);
                    true
                } else {
                    false
                };

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

        // ── 7. Handle flush on misprediction/serializing ───────────────
        if let Some(keep_tag) = flush_keep_tag {
            cpu.stats.stalls_control += 1;
            cpu.stats.pipeline_flushes += 1;
            rename_output.clear();

            // Check whether keep_tag is still in the ROB. It may have been
            // committed in step 1 of this same tick (e.g. a CSR that both
            // commits and triggers a redirect in the same cycle). If it was
            // already committed, flush ALL in-flight entries (they are all
            // younger than the committed instruction).
            let keep_in_rob = self.rob.find_entry(keep_tag).is_some();

            if keep_in_rob {
                // Count squashed entries for misprediction penalty stat
                let squashed: u64 = self.rob.iter_after(keep_tag).count() as u64;
                cpu.stats.misprediction_penalty += squashed;
                // Reclaim physical registers for squashed ROB entries before flushing
                for entry in self.rob.iter_after(keep_tag) {
                    self.free_list.reclaim(entry.phys_dst);
                }
                // Use flush_after (NOT flush) — older IQ entries that haven't
                // been issued yet (waiting on operands) must survive; flushing
                // them would leave their ROB slots stuck in Issued forever,
                // deadlocking the pipeline.
                self.issue_queue.flush_after(keep_tag);
                self.rob.flush_after(keep_tag);
                self.store_buffer.flush_after(keep_tag);
                self.load_queue.flush_after(keep_tag);
            } else {
                // keep_tag was already committed — flush ALL in-flight entries.
                // Reclaim physical registers for all remaining ROB entries.
                for entry in self.rob.iter_all() {
                    self.free_list.reclaim(entry.phys_dst);
                }
                cpu.stats.misprediction_penalty += self.rob.len() as u64;
                self.issue_queue.flush();
                self.rob.flush_all();
                self.store_buffer.flush_speculative();
                self.load_queue.flush();
            }
            // Filter stale (wrong-path) entries from inter-stage latches and pending.
            self.mem1_mem2.retain(|e| e.rob_tag.0 <= keep_tag.0);
            self.mem2_wb.retain(|e| e.rob_tag.0 <= keep_tag.0);
            self.pending_results
                .retain(|p| p.entry.rob_tag.0 <= keep_tag.0);
            // Rebuild speculative rename map from committed map + surviving ROB
            self.rebuild_rename_map();
            // Scoreboard is still used by in-order; rebuild from remaining ROB entries
            self.scoreboard.rebuild_from_rob(&self.rob);
        }

        // ── 8. Dispatch from rename into issue queue ───────────────────
        if flush_keep_tag.is_none() {
            let entries = std::mem::take(rename_output);
            for entry in entries {
                let ok = self
                    .issue_queue
                    .dispatch(entry, &self.rob, cpu, Some(&self.prf));
                debug_assert!(ok, "IQ dispatch failed — rename budget should prevent this");
            }
        }
    }

    fn can_accept(&self) -> usize {
        let rob_free = self.rob.free_slots();
        let sb_free = self.store_buffer.free_slots();
        let lq_free = self.load_queue.free_slots();
        let iq_free = self.issue_queue.available_slots();
        let prf_free = self.free_list.available();
        rob_free
            .min(sb_free)
            .min(lq_free)
            .min(iq_free)
            .min(prf_free)
            .min(self.width)
    }

    fn flush(&mut self, _cpu: &mut Cpu) {
        // Reclaim all phys_dst regs for every in-flight ROB entry
        for entry in self.rob.iter_all() {
            self.free_list.reclaim(entry.phys_dst);
        }
        // Restore speculative rename map to committed state
        self.rename_map = self.committed_rename_map.clone();

        self.rob.flush_all();
        self.store_buffer.flush_speculative();
        self.load_queue.flush();
        self.scoreboard.flush();
        self.issue_queue.flush();
        self.pending_results.clear();
        self.execute_mem1.clear();
        self.mem1_mem2.clear();
        self.mem2_wb.clear();
    }

    fn read_csr_speculative(&self, cpu: &crate::core::Cpu, addr: u32) -> u64 {
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

    fn free_list_mut(&mut self) -> &mut FreeList {
        &mut self.free_list
    }

    fn load_queue_mut(&mut self) -> Option<&mut LoadQueue> {
        Some(&mut self.load_queue)
    }

    fn has_prf(&self) -> bool {
        true
    }
}
