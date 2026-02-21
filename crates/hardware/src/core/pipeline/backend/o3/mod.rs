//! Out-of-Order (O3) backend: issue queue with wakeup/select, out-of-order execution.
//!
//! The O3 backend reuses shared pipeline stages (Memory1, Memory2, Writeback,
//! Commit) and shared hardware units (ALU, FPU, BRU), but has its own:
//! - **IssueQueue**: CAM-style with wakeup/select (vs FIFO for in-order)
//! - **execute_one()**: single-instruction execute (vs batch for in-order)

pub mod execute;
pub mod issue_queue;

use crate::config::Config;
use crate::core::Cpu;
use crate::core::pipeline::backend::shared::{commit, memory1, memory2, writeback};
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry, Mem2WbEntry, RenameIssueEntry};
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::store_buffer::StoreBuffer;

use self::issue_queue::IssueQueue;

/// Out-of-order execution engine.
pub struct O3Engine {
    /// Reorder buffer.
    pub rob: Rob,
    /// Store buffer.
    pub store_buffer: StoreBuffer,
    /// Tag-based register scoreboard.
    pub scoreboard: Scoreboard,
    /// CAM-style issue queue with wakeup/select.
    pub issue_queue: IssueQueue,
    /// Pipeline width (max instructions issued/committed per cycle).
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

impl O3Engine {
    /// Creates a new O3 engine from config.
    pub fn new(config: &Config) -> Self {
        let iq_size = config.pipeline.issue_queue_size;
        Self {
            rob: Rob::new(config.pipeline.rob_size),
            store_buffer: StoreBuffer::new(config.pipeline.store_buffer_size),
            scoreboard: Scoreboard::new(),
            issue_queue: IssueQueue::new(iq_size),
            width: config.pipeline.width,
            execute_mem1: Vec::with_capacity(config.pipeline.width),
            mem1_mem2: Vec::with_capacity(config.pipeline.width),
            mem2_wb: Vec::with_capacity(config.pipeline.width),
            mem1_stall: 0,
        }
    }
}

impl ExecutionEngine for O3Engine {
    fn tick(&mut self, cpu: &mut Cpu, rename_output: &mut Vec<RenameIssueEntry>) {
        // Backend stages run in reverse order (drain from commit to issue)

        let pc_before_commit = cpu.pc;

        // ── 1. Commit ──────────────────────────────────────────────────
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
        // wakeup dependents in the issue queue.
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
                (wb.rob_tag, val)
            })
            .collect();

        writeback::writeback_stage(cpu, &mut self.mem2_wb, &mut self.rob);

        // Broadcast wakeups to issue queue
        for (tag, val) in &wb_wakeups {
            self.issue_queue.wakeup(*tag, *val);
        }

        // ── 3. Memory2 ────────────────────────────────────────────────
        memory2::memory2_stage(
            cpu,
            &mut self.mem1_mem2,
            &mut self.mem2_wb,
            &mut self.store_buffer,
            &mut self.rob,
        );

        // ── 4. Memory1 ────────────────────────────────────────────────
        if self.mem1_stall > 0 {
            self.mem1_stall -= 1;
            cpu.stats.stalls_mem += 1;
        } else {
            memory1::memory1_stage(
                cpu,
                &mut self.execute_mem1,
                &mut self.mem1_mem2,
                &mut self.mem1_stall,
            );
        }

        // ── 5. Backpressure check ──────────────────────────────────────
        let backpressured = !self.execute_mem1.is_empty();

        if cpu.trace && (backpressured || self.mem1_stall > 0) {
            eprintln!(
                "BE  backpressure={} mem1_stall={} ex_mem1={} iq={}",
                backpressured,
                self.mem1_stall,
                self.execute_mem1.len(),
                self.issue_queue.available_slots()
            );
        }

        // ── 6. Issue + Execute ─────────────────────────────────────────
        let mut needs_flush = false;

        if !backpressured {
            let issued = self
                .issue_queue
                .select(self.width, &self.store_buffer, &self.rob);

            if issued.is_empty() && self.issue_queue.len() > 0 {
                cpu.stats.stalls_data += 1;
            }

            for entry in issued {
                let (ex_result, flush) = execute::execute_one(cpu, entry, &mut self.rob);

                // For non-memory instructions, the result is final at execute time.
                // Mark ROB entry Completed immediately and broadcast wakeup so that:
                // 1. New dispatches see the result at resolve_operand time.
                // 2. IQ dependents are woken up with zero additional latency.
                // (Loads get their result from memory2; stores need store buffer
                // resolution. Faulted entries are already marked in the ROB by
                // execute_one, and rob.complete() is a no-op for Faulted state.)
                if !ex_result.ctrl.mem_read && !ex_result.ctrl.mem_write {
                    let val = if ex_result.ctrl.jump {
                        ex_result.pc.wrapping_add(ex_result.inst_size)
                    } else {
                        ex_result.alu
                    };
                    self.rob.complete(ex_result.rob_tag, val);
                    self.issue_queue.wakeup(ex_result.rob_tag, val);
                }

                self.execute_mem1.push(ex_result);

                if flush {
                    needs_flush = true;
                    break; // Stop issuing after serializing/flushing instruction
                }
            }
        }

        // ── 7. Handle flush on misprediction/serializing ───────────────
        if needs_flush {
            rename_output.clear();
            self.mem1_stall = 0;

            if let Some(last) = self.execute_mem1.last() {
                let keep_tag = last.rob_tag;
                // Use flush_after (NOT flush) — older IQ entries that haven't
                // been issued yet (waiting on operands) must survive; flushing
                // them would leave their ROB slots stuck in Issued forever,
                // deadlocking the pipeline.
                self.issue_queue.flush_after(keep_tag);
                self.rob.flush_after(keep_tag);
                self.store_buffer.flush_after(keep_tag);
                // Filter stale (wrong-path) entries from inter-stage latches.
                // Their ROB entries were just flushed, so writeback would
                // silently ignore them, but they waste pipeline bandwidth and
                // can delay valid entries behind them.
                self.mem1_mem2.retain(|e| e.rob_tag.0 <= keep_tag.0);
                self.mem2_wb.retain(|e| e.rob_tag.0 <= keep_tag.0);
            }
            self.scoreboard.rebuild_from_rob(&self.rob);
        }

        // ── 8. Dispatch from rename into issue queue ───────────────────
        if !needs_flush {
            let entries = std::mem::take(rename_output);
            for entry in entries {
                let ok = self.issue_queue.dispatch(entry, &self.rob, cpu);
                debug_assert!(ok, "IQ dispatch failed — rename budget should prevent this");
            }
        }
    }

    fn can_accept(&self) -> usize {
        let rob_free = self.rob.free_slots();
        let sb_free = self.store_buffer.free_slots();
        let iq_free = self.issue_queue.available_slots();
        rob_free.min(sb_free).min(iq_free).min(self.width)
    }

    fn flush(&mut self, _cpu: &mut Cpu) {
        self.rob.flush_all();
        self.store_buffer.flush_speculative();
        self.scoreboard.flush();
        self.issue_queue.flush();
        self.execute_mem1.clear();
        self.mem1_mem2.clear();
        self.mem2_wb.clear();
        self.mem1_stall = 0;
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
}
