//! In-order backend: FIFO issue, single execution path.
//!
//! This backend implements the simple in-order pipeline with:
//! - InOrderIssueUnit: FIFO pass-through (no reordering)
//! - InOrderExecuteUnit: Single ALU/FPU/BRU execution

pub mod execute;
pub mod issue;

use crate::config::Config;
use crate::core::Cpu;
use crate::core::pipeline::backend::shared::{commit, memory1, memory2, writeback};
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry, Mem2WbEntry, RenameIssueEntry};
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::store_buffer::StoreBuffer;

use self::issue::InOrderIssueUnit;

/// In-order execution engine.
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
        }
    }
}

impl ExecutionEngine for InOrderEngine {
    fn tick(&mut self, cpu: &mut Cpu, rename_output: &mut Vec<RenameIssueEntry>) {
        // Backend stages run in reverse order (drain from commit to issue)

        // Commit: retire from ROB head
        let trap_event = commit::commit_stage(
            cpu,
            &mut self.rob,
            &mut self.store_buffer,
            &mut self.scoreboard,
            self.width,
        );

        // Handle trap: flush everything
        if let Some((trap, pc)) = trap_event {
            if cpu.trace {
                eprintln!("BE  * HANDLING TRAP: {:?} at PC {:#x}", trap, pc);
            }
            self.flush(cpu);
            cpu.trap(trap, pc);
            return;
        }

        // Writeback: mark ROB entries as Completed
        writeback::writeback_stage(cpu, &mut self.mem2_wb, &mut self.rob);

        // Memory2: D-cache access / store buffer resolution
        memory2::memory2_stage(
            cpu,
            &mut self.mem1_mem2,
            &mut self.mem2_wb,
            &mut self.store_buffer,
            &mut self.rob,
        );

        // Memory1: address translation (gated by mem1_stall)
        if self.mem1_stall > 0 {
            self.mem1_stall -= 1;
        } else {
            memory1::memory1_stage(
                cpu,
                &mut self.execute_mem1,
                &mut self.mem1_mem2,
                &mut self.mem1_stall,
            );
        }

        // Backpressure: if M1 hasn't consumed previous results, skip issue+execute
        // to prevent new results from overwriting held entries.
        let backpressured = !self.execute_mem1.is_empty();

        if cpu.trace && (backpressured || self.mem1_stall > 0) {
            eprintln!(
                "BE  backpressure={} mem1_stall={} ex_mem1={} iq={}",
                backpressured,
                self.mem1_stall,
                self.execute_mem1.len(),
                self.issuer.available_slots()
            );
        }

        // Issue + Execute: select and read operands via tags
        let (results, needs_flush) = if backpressured {
            (Vec::new(), false)
        } else {
            let issued = self.issuer.select(self.width, &self.rob, cpu);
            execute::execute_inorder(cpu, issued, &mut self.rob)
        };
        self.execute_mem1.extend(results);

        // If execute detected a misprediction / CSR / MRET / SRET / FENCE.I,
        // flush the issue queue, any pending rename output, and the inter-stage
        // latches between execute and the already-processed backend stages.
        // The frontend latches will be flushed by Pipeline::tick() since execute
        // updated cpu.pc. We flush here so that wrongly-fetched instructions
        // don't continue flowing through the backend.
        if needs_flush {
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

        // Accept dispatched instructions from rename.
        // Skip dispatch when backpressured: if issue+execute can't run,
        // dispatching would fill the ROB/issue queue with instructions whose
        // operands can never be resolved (in-order deadlock).
        if !needs_flush && !backpressured {
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

    fn flush(&mut self, _cpu: &mut Cpu) {
        self.rob.flush_all();
        self.store_buffer.flush_speculative();
        self.scoreboard.flush();
        self.issuer.flush();
        self.execute_mem1.clear();
        self.mem1_mem2.clear();
        self.mem2_wb.clear();
        self.mem1_stall = 0;
    }

    fn read_csr_speculative(&self, cpu: &crate::core::Cpu, addr: u32) -> u64 {
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
