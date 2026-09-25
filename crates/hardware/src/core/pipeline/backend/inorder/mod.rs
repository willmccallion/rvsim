//! In-order backend: FIFO issue, single execution path.
//!
//! Implements the simple in-order pipeline with:
//! - [`InOrderIssueUnit`] — FIFO pass-through (no reordering).
//! - `InOrderExecuteUnit` — single ALU/FPU/BRU execution.
//!
//! The engine owns its in-flight memory bookkeeping
//! (`BackendCommon` = mailbox + `outstanding_loads` / stores / walks /
//! fetches + `next_req_id` + routing IDs). Memory1 runs inside `tick`,
//! reaching the outstanding tables and the event queue directly.

pub mod execute;
pub mod issue;

use crate::config::Config;
use crate::sim::SimState;
use crate::core::pipeline::backend::shared::{commit, memory1, memory2, writeback};
use crate::core::pipeline::engine::{BackendCommon, ExecutionEngine};
use crate::core::pipeline::free_list::FreeList;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry, Mem2WbEntry, RenameIssueEntry};
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::rename_map::RenameMap;
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::core::units::bru::BranchPredictor;
use crate::sim::components::{CacheId, PipelineId};
use crate::sim::stats::paths;

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
    /// Execute → Memory1 latch.
    pub execute_mem1: Vec<ExMem1Entry>,
    /// Memory1 → Memory2 latch.
    pub mem1_mem2: Vec<Mem1Mem2Entry>,
    /// Memory2 → Writeback latch.
    pub mem2_wb: Vec<Mem2WbEntry>,
    /// In-flight memory bookkeeping: mailbox + outstanding tables + routing IDs.
    pub common: BackendCommon,
    /// Current cycle counter (used for debug stats; the simulator's cycle
    /// drives all timing).
    cycle: u64,
    /// Committed rename map stub (unused; required by shared `commit_stage` signature).
    committed_rename_map: RenameMap,
    /// Free list stub (unused; required by shared `commit_stage` signature).
    free_list: FreeList<PhysReg>,
}

impl InOrderEngine {
    /// Creates a new in-order engine from config and routing IDs.
    pub fn new(
        config: &Config,
        pipeline_id: PipelineId,
        l1_i_id: CacheId,
        l1_d_id: CacheId,
    ) -> Self {
        let common = BackendCommon {
            pipeline_id,
            l1_i_id,
            l1_d_id,
            ..BackendCommon::default()
        };
        Self {
            rob: Rob::new(config.pipeline.rob_size),
            store_buffer: StoreBuffer::new(config.pipeline.store_buffer_size),
            scoreboard: Scoreboard::new(),
            issuer: InOrderIssueUnit::new(config.pipeline.rob_size),
            width: config.pipeline.width,
            execute_mem1: Vec::with_capacity(config.pipeline.width),
            mem1_mem2: Vec::with_capacity(config.pipeline.width),
            mem2_wb: Vec::with_capacity(config.pipeline.width),
            common,
            cycle: 0,
            committed_rename_map: RenameMap::new(),
            free_list: FreeList::new(0, 0),
        }
    }
}

impl ExecutionEngine for InOrderEngine {
    fn tick(
        &mut self,
        state: &mut SimState,
        rename_output: &mut Vec<RenameIssueEntry>,
        redirect_pending: &mut bool,
    ) {
        self.cycle += 1;

        let pc_before_commit = state.hart.pc;

        let trap_event = commit::commit_stage(
            state,
            &mut self.common,
            &mut self.rob,
            &mut self.store_buffer,
            &mut self.scoreboard,
            &mut self.committed_rename_map,
            &mut self.free_list,
            self.width,
            None,
            None,
            None,
            None,
            None,
            None,
            redirect_pending,
        );

        if let Some((trap, pc)) = trap_event {
            self.flush(state);
            *redirect_pending = true;
            state.trap(&trap, pc);
            state.hart.committed_next_pc = state.hart.pc;
            return;
        }

        // MRET/SRET changed the PC; flush so post-redirect stale fetches don't proceed.
        if state.hart.pc != pc_before_commit {
            self.flush(state);
            rename_output.clear();
            return;
        }

        writeback::writeback_stage(state, &mut self.mem2_wb, &mut self.rob);

        let _ = memory2::memory2_stage(
            state,
            &mut self.mem1_mem2,
            &mut self.mem2_wb,
            &mut self.store_buffer,
            None,
            None,
        );

        // Memory1 consumes execute_mem1, emits MemReq packets for misses,
        // and parks parked loads into self.common.outstanding_loads. SB
        // forwards and stores resolve straight into mem1_mem2.
        let mut input = std::mem::take(&mut self.execute_mem1);
        memory1::memory1_stage(state, self, &mut input);
        // Ops behind an unresolved translation walk go back; ops waiting on
        // a store-buffer drain live in `common.mem1_replay`.
        self.execute_mem1.extend(input);

        // Skip issue+execute when M1 hasn't drained, so we don't overwrite held entries.
        let backpressured = !self.execute_mem1.is_empty();

        let (results, needs_flush) = if backpressured {
            (Vec::new(), false)
        } else {
            let issued = self.issuer.select(self.width, &self.rob, &self.store_buffer, state);
            if issued.is_empty() && !self.issuer.is_empty() {
                state.stats.counter(paths::core::pipeline::STALLS_DATA).inc();
            }
            // Aggregate in-flight fp_flags so CSR reads of fflags see older FP results.
            let mut inflight_fp_flags: u8 = 0;
            for e in self.execute_mem1.iter().chain(&self.common.mem1_replay) {
                inflight_fp_flags |= e.fp_flags;
            }
            for e in &self.mem1_mem2 {
                inflight_fp_flags |= e.fp_flags;
            }
            for e in &self.mem2_wb {
                inflight_fp_flags |= e.fp_flags;
            }
            execute::execute_inorder(state, issued, &mut self.rob, inflight_fp_flags, redirect_pending)
        };
        self.execute_mem1.extend(results);

        if needs_flush {
            state.stats.counter(paths::core::pipeline::STALLS_CONTROL).inc();
            state.stats.counter(paths::core::pipeline::FLUSHES_TOTAL).inc();
            self.issuer.flush();
            rename_output.clear();
            if let Some(last) = self.execute_mem1.last() {
                let keep_tag = last.rob_tag;
                self.rob.flush_after(keep_tag);
                self.store_buffer.flush_after(keep_tag);
                self.common.squash_after(keep_tag);
            }
            self.scoreboard.rebuild_from_rob(&self.rob);
        }

        // Dispatch even during backpressure: skipping it lets rename_output outgrow issue capacity.
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

    fn flush(&mut self, state: &mut SimState) {
        self.rob.flush_all();
        self.store_buffer.flush_speculative();
        self.scoreboard.flush();
        self.issuer.flush();
        self.execute_mem1.clear();
        self.common.mem1_replay.clear();
        self.mem1_mem2.clear();
        self.mem2_wb.clear();
        state.core.branch_predictor.repair_to_committed();
    }

    fn read_csr_speculative(&self, state: &crate::sim::SimState, addr: crate::common::CsrAddr) -> u64 {
        // In-order serialization commits older CSR writes before any CSR read issues.
        state.csr_read(addr)
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

    fn execute_mem1_mut(&mut self) -> &mut Vec<ExMem1Entry> {
        &mut self.execute_mem1
    }

    fn mem1_mem2_mut(&mut self) -> &mut Vec<Mem1Mem2Entry> {
        &mut self.mem1_mem2
    }

    fn common(&self) -> &BackendCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut BackendCommon {
        &mut self.common
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_inorder_engine_new() {
        let config = Config::default();
        let engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        assert_eq!(engine.width, config.pipeline.width);
    }

    #[test]
    fn test_inorder_engine_flush() {
        let config = Config::default();
        let mut engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        let mut state = SimState::build(&config, "");

        engine.flush(&mut state);

        assert_eq!(engine.execute_mem1.len(), 0);
        assert_eq!(engine.mem1_mem2.len(), 0);
        assert_eq!(engine.mem2_wb.len(), 0);
    }

    #[test]
    fn test_inorder_engine_can_accept() {
        let config = Config::default();
        let engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        assert_eq!(engine.can_accept(), engine.width);
    }

    #[test]
    fn test_inorder_engine_read_csr_speculative() {
        let config = Config::default();
        let engine =
            InOrderEngine::new(&config, PipelineId::new(0), CacheId::new(0), CacheId::new(1));
        let mut state = SimState::build(&config, "");

        state.csr_write(crate::core::arch::csr::MSCRATCH, 0x1234);
        assert_eq!(engine.read_csr_speculative(&state, crate::core::arch::csr::MSCRATCH), 0x1234);
    }
}
