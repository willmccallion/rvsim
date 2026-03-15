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
use crate::core::pipeline::signals::ControlFlow;
use crate::core::pipeline::store_buffer::StoreBuffer;

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
            load_ports: config.pipeline.load_ports,
            store_ports: config.pipeline.store_ports,
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
        use crate::common::RegIdx;
        use crate::core::pipeline::prf::PhysReg;
        // GPRs: arch reg i → PhysReg(i), skip x0 (hardwired zero)
        for i in 1u8..32 {
            let val = cpu.regs.read(RegIdx::new(i));
            if val != 0 {
                self.prf.write(PhysReg(i as u16), val);
            }
        }
        // FPRs: arch reg i → PhysReg(32 + i)
        for i in 0u8..32 {
            let val = cpu.regs.read_f(RegIdx::new(i));
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
            if entry.ctrl.reg_write && !entry.rd.is_zero() {
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
            Some(&mut self.prf),
        );

        // Handle trap: flush everything
        if let Some((trap, pc)) = trap_event {
            self.flush(cpu);
            cpu.redirect_pending = true;
            cpu.trap(&trap, pc);
            cpu.committed_next_pc = cpu.pc;
            return;
        }

        // Handle MRET/SRET redirect
        if cpu.pc != pc_before_commit {
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
                } else if wb.ctrl.control_flow == ControlFlow::Jump {
                    wb.pc.wrapping_add(wb.inst_size.as_u64())
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

        // ── 2b. MSHR completions ─────────────────────────────────────
        // Drain completed MSHRs: install cache lines in L1D and resume
        // parked loads/atomics into the mem1→mem2 latch.
        if cpu.l1d_mshrs.capacity() > 0 {
            let completed = cpu.l1d_mshrs.drain_completions(now);
            for mshr_entry in completed {
                // Install the fetched line into L1D (with eviction tracking)
                let (_penalty, evicted) = cpu.l1_d_cache.install_line_public_tracked(
                    mshr_entry.line_addr,
                    mshr_entry.is_write,
                    0, // write-back penalty already accounted for in miss latency
                );

                // Exclusive policy: L1D eviction → install evicted line into L2
                if cpu.inclusion_policy == crate::config::InclusionPolicy::Exclusive
                    && cpu.l2_cache.enabled
                    && let Some(ev) = evicted
                {
                    let _ = cpu.l2_cache.install_or_replace(ev.addr, ev.dirty, 0);
                    cpu.stats.exclusive_l1_to_l2_swaps += 1;
                }

                // Resume parked loads/atomics
                for waiter in mshr_entry.waiters {
                    if let Some(mut parked) = waiter.parked_entry {
                        // Set the completion cycle to now (data just arrived)
                        parked.complete_cycle = now;
                        self.mem1_mem2.push(parked);
                    }
                }
            }
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
            // Find the violating load's PC from the ROB
            let violation_pc = self.rob.find_entry(violating_tag).map_or(cpu.pc, |e| e.pc);

            // keep_tag = tag before the violating load (everything older is kept).
            // We need a tag that is actually present in the ROB so that
            // flush_after() can locate it. The previous approach of
            // `violating_tag - 1` created a synthetic tag that may have
            // already been committed and removed from the ROB, causing
            // flush_after() to silently no-op while free_list.reclaim()
            // had already freed the physical registers — a use-after-free
            // that led to register aliasing corruption.
            let keep_tag = self.rob.prev_tag_of(violating_tag);

            cpu.stats.mem_ordering_violations += 1;
            cpu.stats.pipeline_flushes += 1;
            cpu.stats.stalls_control += 1;

            if let Some(keep_tag) = keep_tag {
                // Reclaim physical registers for squashed entries
                for entry in self.rob.iter_after(keep_tag) {
                    self.free_list.reclaim(entry.phys_dst);
                }
                let squashed = self.rob.iter_after(keep_tag).count() as u64;
                cpu.stats.misprediction_penalty += squashed;

                self.issue_queue.flush_after(keep_tag);
                self.rob.flush_after(keep_tag);
                self.store_buffer.flush_after(keep_tag);
                self.load_queue.flush_after(keep_tag);
                cpu.l1d_mshrs.flush_after(keep_tag);

                self.mem1_mem2.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
                self.mem2_wb.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
                self.pending_results
                    .retain(|p| p.entry.rob_tag.is_older_or_eq(keep_tag));
                self.execute_mem1.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
            } else {
                // The violating load is at the ROB head (no preceding entry),
                // or the preceding entry was already committed. Full flush.
                for entry in self.rob.iter_all() {
                    self.free_list.reclaim(entry.phys_dst);
                }
                let squashed = self.rob.len() as u64;
                cpu.stats.misprediction_penalty += squashed;

                self.issue_queue.flush();
                self.rob.flush_all();
                self.store_buffer.flush_speculative();
                self.load_queue.flush();
                cpu.l1d_mshrs.flush();

                self.mem1_mem2.clear();
                self.mem2_wb.clear();
                self.pending_results.clear();
                self.execute_mem1.clear();
            }

            self.rebuild_rename_map();
            self.scoreboard.rebuild_from_rob(&self.rob);

            cpu.pc = violation_pc;
            cpu.redirect_pending = true;
            rename_output.clear();
            return;
        }

        // ── 4. Memory1 ────────────────────────────────────────────────
        // With MSHRs: misses are parked in MSHRs so memory1 can always
        // accept new entries. Without MSHRs: gate on the oldest incomplete
        // memory entry (blocking behavior).
        let has_mshrs = cpu.l1d_mshrs.capacity() > 0;
        let mem1_busy = !has_mshrs
            && self.mem1_mem2.iter().any(|e| {
                let is_mem = e.ctrl.mem_read || e.ctrl.mem_write;
                is_mem && e.complete_cycle > now
            });

        if mem1_busy {
            cpu.stats.stalls_mem += 1;
        } else {
            let cancelled = memory1::memory1_stage(
                cpu,
                &mut self.execute_mem1,
                &mut self.mem1_mem2,
                now,
                Some(&mut self.load_queue),
            );
            // Cancel speculative wakeups for loads that missed L1D
            for phys in cancelled {
                self.issue_queue.cancel_wakeup_phys(phys, &self.prf);
            }
        }

        // ── 5. Backpressure check ──────────────────────────────────────
        // Backpressured if execute_mem1 is not empty (memory pipeline in use)
        // OR if there are pending memory results completing this cycle that will
        // be pushed into execute_mem1 in step 6a.
        // Memory pipeline backpressure: only when execute_mem1 has undrained
        // entries (e.g. MSHR full pushback, trap). Pending results in
        // pending_results are drained to execute_mem1 at step 6a AFTER issue
        // at step 6, so there's no conflict — the Mem FU is pipelined and
        // accepts a new op every cycle.
        let mem_backpressured = !self.execute_mem1.is_empty();

        if mem_backpressured {
            cpu.stats.stalls_backpressure += 1;
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
                        let val = if entry.ctrl.control_flow == ControlFlow::Jump {
                            entry.pc.wrapping_add(entry.inst_size.as_u64())
                        } else {
                            entry.alu
                        };
                        // Store fp_flags in the ROB now (same rationale as the
                        // pipelined path — younger CSR reads need to see them).
                        if entry.fp_flags != 0 {
                            self.rob.set_fp_flags(entry.rob_tag, entry.fp_flags);
                        }
                        if let Some(info) = entry.sfence_vma {
                            self.rob.set_sfence_vma(entry.rob_tag, info);
                        }
                        // CSR writes are deferred to commit (shared/commit.rs)
                        // to prevent speculative CSR state from persisting if
                        // an older instruction traps.
                        self.rob.complete(entry.rob_tag, val);
                        self.prf.write(entry.rd_phys, val);
                        self.issue_queue.wakeup_phys(entry.rd_phys, val);
                        // ROB Completed, PRF written, wakeup done — skip memory pipeline.
                    } else {
                        // Pipelined non-memory: ROB already Completed, PRF written,
                        // wakeup broadcast at issue time — no need to traverse the
                        // memory pipeline (memory1 → memory2 → writeback). Commit
                        // will retire directly from the ROB.
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

            for entry in issued {
                let fu_type = FuType::classify(&entry.ctrl);

                // Memory pipeline backpressure: block memory ops from issuing
                // when the memory pipeline is busy, but allow non-memory ops
                // (ALU, branch) to continue issuing freely.
                if mem_backpressured && fu_type == FuType::Mem {
                    let ok = self.issue_queue.dispatch(entry, &self.rob, cpu, Some(&self.prf));
                    debug_assert!(ok, "re-dispatch after mem backpressure failed");
                    continue;
                }

                // Check for structural hazard (no free FU of required type)
                if !self.fu_pool.has_free(fu_type, now) {
                    cpu.stats.stalls_fu_structural += 1;
                    stalled_fu = true;
                    // Leave entry in IQ (select already removed it — re-dispatch needed)
                    // For simplicity: re-dispatch back into IQ
                    let ok = self.issue_queue.dispatch(entry, &self.rob, cpu, Some(&self.prf));
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
                    let val = if ex_result.ctrl.control_flow == ControlFlow::Jump {
                        ex_result.pc.wrapping_add(ex_result.inst_size.as_u64())
                    } else {
                        ex_result.alu
                    };
                    // Store fp_flags in the ROB now so that a younger serializing
                    // CSR instruction (e.g. fsflags) sees them when it drains
                    // older entries at execute time.  Commit will apply them to
                    // the architectural fflags register in program order.
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

                // Speculative load wakeup: when a load issues and MSHRs are
                // available, optimistically wake dependents assuming L1D hit.
                // PRF is NOT written — select() validates against PRF.
                // If the load hits L1D, writeback confirms with the real value.
                // If it misses, memory1 calls cancel_wakeup_phys().
                let is_load = ex_result.ctrl.mem_read && !ex_result.ctrl.mem_write;
                if is_load && ex_result.trap.is_none() && cpu.l1d_mshrs.capacity() > 0 {
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
                cpu.l1d_mshrs.flush_after(keep_tag);
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
                cpu.l1d_mshrs.flush();
            }
            // Filter stale (wrong-path) entries from inter-stage latches and pending.
            self.mem1_mem2.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
            self.mem2_wb.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
            self.pending_results.retain(|p| p.entry.rob_tag.is_older_or_eq(keep_tag));
            self.execute_mem1.retain(|e| e.rob_tag.is_older_or_eq(keep_tag));
            // Rebuild speculative rename map from committed map + surviving ROB
            self.rebuild_rename_map();
            // Scoreboard is still used by in-order; rebuild from remaining ROB entries
            self.scoreboard.rebuild_from_rob(&self.rob);
        }

        // ── 8. Dispatch from rename into issue queue ───────────────────
        if flush_keep_tag.is_none() {
            let entries = std::mem::take(rename_output);
            for entry in entries {
                let ok = self.issue_queue.dispatch(entry, &self.rob, cpu, Some(&self.prf));
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
        rob_free.min(sb_free).min(lq_free).min(iq_free).min(prf_free).min(self.width)
    }

    fn flush(&mut self, cpu: &mut Cpu) {
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
        // Flush all MSHRs — their parked entries are now invalid
        cpu.l1d_mshrs.flush();

        // Invariant check: after a full flush the total number of physical
        // registers must be conserved.  Every phys reg is either free OR
        // referenced by the committed rename map (one unique phys reg per
        // architectural register).
        debug_assert_eq!(
            self.free_list.available() + 64, // 32 GPR + 32 FPR mapped
            self.prf.capacity(),
            "PRF register leak detected: free={} + 64 mapped != {} total",
            self.free_list.available(),
            self.prf.capacity(),
        );
    }

    fn read_csr_speculative(&self, cpu: &crate::core::Cpu, addr: crate::common::CsrAddr) -> u64 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::RegIdx;
    use crate::config::Config;
    use crate::soc::builder::System;

    #[test]
    fn test_o3_engine_new_and_flush() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        let mut engine = O3Engine::new(&config);
        assert_eq!(engine.width, config.pipeline.width);

        engine.flush(&mut cpu);
        assert_eq!(engine.execute_mem1.len(), 0);
    }

    #[test]
    fn test_o3_engine_sync_arch_regs() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
        let mut engine = O3Engine::new(&config);

        cpu.regs.write(RegIdx::new(1), 42);
        engine.sync_arch_regs(&cpu);

        assert_eq!(engine.prf.read(crate::core::pipeline::prf::PhysReg(1)), 42);
    }
}
