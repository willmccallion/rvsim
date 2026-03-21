//! In-order backend: FIFO issue, single execution path.
//!
//! This backend implements the simple in-order pipeline with:
//! - `InOrderIssueUnit`: FIFO pass-through (no reordering)
//! - `InOrderExecuteUnit`: Single ALU/FPU/BRU execution

pub mod execute;
pub mod issue;

use crate::config::Config;
use crate::core::Cpu;
use crate::core::pipeline::backend::shared::{commit, memory1, memory2, writeback};
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::free_list::FreeList;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry, Mem2WbEntry, RenameIssueEntry};
use crate::core::pipeline::rename_map::RenameMap;
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::core::units::bru::BranchPredictor;

/// Drain completed MSHRs: install cache lines in L1D and resume parked
/// loads/atomics into the mem1→mem2 latch.  Mirrors the O3 backend's
/// MSHR completion logic but without PRF/wakeup handling.
fn drain_mshr_completions(cpu: &mut Cpu, mem1_mem2: &mut Vec<Mem1Mem2Entry>, now: u64) {
    if cpu.l1d_mshrs.capacity() == 0 {
        return;
    }
    let completed = cpu.l1d_mshrs.drain_completions(now);
    for mshr_entry in completed {
        let (_penalty, evicted) = cpu.l1_d_cache.install_line_public_tracked(
            mshr_entry.line_addr,
            mshr_entry.is_write,
            0,
        );

        if cpu.inclusion_policy == crate::config::InclusionPolicy::Exclusive
            && cpu.l2_cache.enabled
            && let Some(ev) = evicted
        {
            let _ = cpu.l2_cache.install_or_replace(ev.addr, ev.dirty, 0);
            cpu.stats.exclusive_l1_to_l2_swaps += 1;
        }

        for waiter in mshr_entry.waiters {
            if let Some(mut parked) = waiter.parked_entry {
                parked.complete_cycle = now;
                mem1_mem2.push(parked);
            }
        }
    }
}

use self::issue::InOrderIssueUnit;

/// In-order execution engine.
#[derive(Debug)]
pub struct InOrderEngine {
    /// Reorder buffer.
    pub rob: Rob,
    /// Store buffer.
    pub store_buffer: StoreBuffer,
    /// Tag-based register scoreboard.
    pub scoreboard: Scoreboard,
    /// FIFO issue unit.
    pub issuer: InOrderIssueUnit,
    /// Pipeline width.
    pub width: usize,
    /// Execute -> Memory1 latch.
    pub execute_mem1: Vec<ExMem1Entry>,
    /// Memory1 -> Memory2 latch.
    pub mem1_mem2: Vec<Mem1Mem2Entry>,
    /// Memory2 -> Writeback latch.
    pub mem2_wb: Vec<Mem2WbEntry>,
    /// Memory1 stall counter (D-TLB / D-cache latency).
    pub mem1_stall: u64,
    /// Current cycle counter (for MSHR completion tracking).
    cycle: u64,
    /// Committed rename map stub (unused; required by shared `commit_stage` signature).
    committed_rename_map: RenameMap,
    /// Free list stub (unused; required by shared `commit_stage` signature).
    free_list: FreeList,
}

impl InOrderEngine {
    /// Creates a new in-order engine from config.
    pub fn new(config: &Config) -> Self {
        Self {
            rob: Rob::new(config.pipeline.rob_size),
            store_buffer: StoreBuffer::new(config.pipeline.store_buffer_size),
            scoreboard: Scoreboard::new(),
            issuer: InOrderIssueUnit::new(config.pipeline.rob_size),
            width: config.pipeline.width,
            execute_mem1: Vec::with_capacity(config.pipeline.width),
            mem1_mem2: Vec::with_capacity(config.pipeline.width),
            mem2_wb: Vec::with_capacity(config.pipeline.width),
            mem1_stall: 0,
            cycle: 0,
            committed_rename_map: RenameMap::new(),
            free_list: FreeList::new(0, 0),
        }
    }
}

impl ExecutionEngine for InOrderEngine {
    fn tick(&mut self, cpu: &mut Cpu, rename_output: &mut Vec<RenameIssueEntry>) {
        // Backend stages run in reverse order (drain from commit to issue)
        self.cycle += 1;

        // Drain completed MSHRs before anything else — parked loads need
        // to re-enter the mem1→mem2 latch so they can complete.
        drain_mshr_completions(cpu, &mut self.mem1_mem2, self.cycle);

        let pc_before_commit = cpu.pc;

        // Commit: retire from ROB head
        let trap_event = commit::commit_stage(
            cpu,
            &mut self.rob,
            &mut self.store_buffer,
            &mut self.scoreboard,
            &mut self.committed_rename_map,
            &mut self.free_list,
            self.width,
            None, // in-order backend: no load queue
            None, // in-order backend: no PRF
        );

        // Handle trap: flush everything
        if let Some((trap, pc)) = trap_event {
            self.flush(cpu);
            cpu.redirect_pending = true;
            cpu.trap(&trap, pc);
            cpu.committed_next_pc = cpu.pc;
            return;
        }

        // Handle MRET/SRET redirect: commit changed the PC, flush the
        // entire backend so stale instructions fetched from the sequential
        // path after MRET/SRET don't continue through the pipeline.
        if cpu.pc != pc_before_commit {
            self.flush(cpu);
            rename_output.clear();
            return;
        }

        // Writeback: mark ROB entries as Completed
        writeback::writeback_stage(cpu, &mut self.mem2_wb, &mut self.rob);

        // Memory2: D-cache access / store buffer resolution
        let _ = memory2::memory2_stage(
            cpu,
            &mut self.mem1_mem2,
            &mut self.mem2_wb,
            &mut self.store_buffer,
            &mut self.rob,
            None, // in-order backend: no load queue
        );

        // Memory1: address translation (gated by mem1_stall)
        if self.mem1_stall > 0 {
            self.mem1_stall -= 1;
            cpu.stats.stalls_mem += 1;
        } else {
            let _ = memory1::memory1_stage(
                cpu,
                &mut self.execute_mem1,
                &mut self.mem1_mem2,
                self.cycle,
                None, // in-order backend: no load queue
            );
            // Derive stall from the worst-case entry's complete_cycle
            self.mem1_stall = self
                .mem1_mem2
                .iter()
                .map(|e| e.complete_cycle.saturating_sub(self.cycle))
                .max()
                .unwrap_or(0);
        }

        // Backpressure: if M1 hasn't consumed previous results, skip issue+execute
        // to prevent new results from overwriting held entries.
        let backpressured = !self.execute_mem1.is_empty();

        // Issue + Execute: select and read operands via tags
        let (results, needs_flush) = if backpressured {
            (Vec::new(), false)
        } else {
            let issued = self.issuer.select(self.width, &self.rob, &self.store_buffer, cpu);
            if issued.is_empty() && !self.issuer.is_empty() {
                cpu.stats.stalls_data += 1;
            }
            // Accumulate fp_flags from in-flight pipeline entries that
            // haven't reached writeback/ROB yet, so CSR reads of fflags
            // see flags from all older FP instructions.
            let mut inflight_fp_flags: u8 = 0;
            for e in &self.execute_mem1 {
                inflight_fp_flags |= e.fp_flags;
            }
            for e in &self.mem1_mem2 {
                inflight_fp_flags |= e.fp_flags;
            }
            for e in &self.mem2_wb {
                inflight_fp_flags |= e.fp_flags;
            }
            execute::execute_inorder(cpu, issued, &mut self.rob, inflight_fp_flags)
        };
        self.execute_mem1.extend(results);

        // If execute detected a misprediction / CSR / MRET / SRET / FENCE.I,
        // flush the issue queue, any pending rename output, and the inter-stage
        // latches between execute and the already-processed backend stages.
        // The frontend latches will be flushed by Pipeline::tick() since execute
        // updated cpu.pc. We flush here so that wrongly-fetched instructions
        // don't continue flowing through the backend.
        if needs_flush {
            cpu.stats.stalls_control += 1;
            cpu.stats.pipeline_flushes += 1;
            self.issuer.flush();
            rename_output.clear();
            // The mem1_stall was from a pre-branch instruction whose results
            // are already in the later stages.  Clear it so that the branch
            // entry in execute_mem1 can drain through M1 immediately, rather
            // than being stuck behind a stale stall that blocks the entire
            // backend via backpressure.
            self.mem1_stall = 0;
            // Keep execute_mem1 (just produced), but flush the rest of the in-flight
            // pipeline — those contain instructions from BEFORE the mispredicted branch
            // that already drained through the backend. They're OK — they're in-order
            // and must have been issued before the branch.
            // However, the ROB may contain entries allocated by rename for instructions
            // that were fetched after the branch. Flush those.
            // In the in-order case, the last entry in execute_mem1 is the branch itself.
            // All ROB entries after this one are speculative and must be flushed.
            if let Some(last) = self.execute_mem1.last() {
                let keep_tag = last.rob_tag;
                self.rob.flush_after(keep_tag);
                // Only flush store buffer entries allocated after the branch.
                // Pre-branch stores may still be in-flight (Ready but not yet
                // Committed) and must be kept for correct store-to-load forwarding.
                self.store_buffer.flush_after(keep_tag);
            }
            // Rebuild scoreboard from surviving ROB entries (pre-branch instructions
            // that haven't committed yet still need their scoreboard entries).
            self.scoreboard.rebuild_from_rob(&self.rob);
        }

        // Accept dispatched instructions from rename into the issue queue.
        // We dispatch even during backpressure — the instructions sit in the
        // FIFO until issue+execute can run again.  Skipping dispatch during
        // backpressure would let rename keep allocating ROB entries (since
        // can_accept() checks the *issue queue* size, not rename_output) while
        // the issue queue stays the same size, eventually causing rename_output
        // to outgrow the remaining issue capacity.  When dispatch finally runs,
        // the excess entries are silently dropped, leaving their ROB slots
        // permanently stuck in Issued state and deadlocking the pipeline.
        if !needs_flush {
            let rename_entries = std::mem::take(rename_output);
            if !rename_entries.is_empty() {
                self.issuer.dispatch(rename_entries);
            }
        }
    }

    fn can_accept(&self) -> usize {
        let rob_free = self.rob.free_slots();
        let sb_free = self.store_buffer.free_slots();
        let issue_free = self.issuer.available_slots();
        rob_free.min(sb_free).min(issue_free).min(self.width)
    }

    fn flush(&mut self, cpu: &mut Cpu) {
        self.rob.flush_all();
        self.store_buffer.flush_speculative();
        self.scoreboard.flush();
        self.issuer.flush();
        self.execute_mem1.clear();
        self.mem1_mem2.clear();
        self.mem2_wb.clear();
        self.mem1_stall = 0;
        // Flush all MSHRs — their parked entries are now invalid
        cpu.l1d_mshrs.flush();
        // Reset speculative GHR to committed state — wrong-path branch
        // outcomes may have been pushed into the speculative history.
        cpu.branch_predictor.repair_to_committed();
    }

    fn read_csr_speculative(&self, cpu: &crate::core::Cpu, addr: crate::common::CsrAddr) -> u64 {
        // Check ROB for pending CsrUpdate entries (newest first)
        // For now, just read the architectural CSR
        // TODO: scan ROB for CsrUpdate with matching addr
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::soc::builder::System;

    #[test]
    fn test_inorder_engine_new() {
        let config = Config::default();
        let engine = InOrderEngine::new(&config);
        assert_eq!(engine.width, config.pipeline.width);
        assert_eq!(engine.mem1_stall, 0);
    }

    #[test]
    fn test_inorder_engine_flush() {
        let config = Config::default();
        let mut engine = InOrderEngine::new(&config);
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        engine.mem1_stall = 5;
        engine.flush(&mut cpu);

        assert_eq!(engine.mem1_stall, 0);
        assert_eq!(engine.execute_mem1.len(), 0);
        assert_eq!(engine.mem1_mem2.len(), 0);
        assert_eq!(engine.mem2_wb.len(), 0);
    }

    #[test]
    fn test_inorder_engine_can_accept() {
        let config = Config::default();
        let engine = InOrderEngine::new(&config);
        assert_eq!(engine.can_accept(), engine.width);
    }

    #[test]
    fn test_inorder_engine_read_csr_speculative() {
        let config = Config::default();
        let engine = InOrderEngine::new(&config);
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        cpu.csr_write(crate::core::arch::csr::MSCRATCH, 0x1234);
        assert_eq!(engine.read_csr_speculative(&cpu, crate::core::arch::csr::MSCRATCH), 0x1234);
    }
}
