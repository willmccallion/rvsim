//! In-Order Issue Unit: FIFO queue with tag-based operand read.
//!
//! For the in-order backend, issue is a FIFO queue. When selecting instructions,
//! the issue stage reads operand values using the tags captured at rename time:
//! - If tag is None → read from architectural register file.
//! - If tag points to a completed ROB entry → bypass the result.
//! - If the ROB entry is still in-flight → stall (operand not ready).

use crate::core::Cpu;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::rob::{Rob, RobState, RobTag};
use std::collections::VecDeque;

/// FIFO issue unit for in-order execution.
pub struct InOrderIssueUnit {
    queue: VecDeque<RenameIssueEntry>,
    capacity: usize,
}

impl InOrderIssueUnit {
    /// Creates a new FIFO issue unit with the given capacity.
    ///
    /// The capacity must be at least as large as the ROB, because during
    /// backend stalls (e.g. M1 cache miss), rename keeps allocating ROB
    /// entries that accumulate in `rename_output`. When the stall ends,
    /// all of these are dispatched at once. If the issue queue is smaller
    /// than the ROB, entries would be silently dropped, leaving ROB slots
    /// permanently stuck in `Issued` state and deadlocking the pipeline.
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Accept dispatched instructions from rename.
    pub fn dispatch(&mut self, entries: Vec<RenameIssueEntry>) {
        for entry in entries {
            if self.queue.len() < self.capacity {
                self.queue.push_back(entry);
            }
        }
    }

    /// Select instructions to execute this cycle, reading operands via
    /// tags captured at rename time. Returns up to `width` entries with
    /// operands populated.
    ///
    /// In-order: if the head-of-queue is blocked, nothing behind it can issue.
    pub fn select(&mut self, width: usize, rob: &Rob, cpu: &Cpu) -> Vec<RenameIssueEntry> {
        let mut selected = Vec::with_capacity(width);

        for _ in 0..width {
            let entry = match self.queue.front() {
                Some(e) => e,
                None => break,
            };

            // Faulted instructions don't need operands — pass through
            if entry.trap.is_some() {
                selected.push(self.queue.pop_front().unwrap());
                continue;
            }

            // Try to read all source operands using tags captured at rename
            let rv1 = read_operand_by_tag(entry.rs1, entry.ctrl.rs1_fp, entry.rs1_tag, rob, cpu);
            let rv2 = read_operand_by_tag(entry.rs2, entry.ctrl.rs2_fp, entry.rs2_tag, rob, cpu);
            let rv3 = if entry.ctrl.rs3_fp {
                read_operand_by_tag(entry.rs3, true, entry.rs3_tag, rob, cpu)
            } else {
                Some(0)
            };

            match (rv1, rv2, rv3) {
                (Some(v1), Some(v2), Some(v3)) => {
                    let mut issued = self.queue.pop_front().unwrap();
                    issued.rv1 = v1;
                    issued.rv2 = v2;
                    issued.rv3 = v3;
                    selected.push(issued);
                }
                _ => {
                    // Head of queue blocked — in-order can't skip
                    if cpu.trace {
                        eprintln!(
                            "IS  pc={:#x} STALL rs1={} rs1_tag={:?}={:?} rs2={} rs2_tag={:?}={:?}",
                            entry.pc, entry.rs1, entry.rs1_tag, rv1, entry.rs2, entry.rs2_tag, rv2,
                        );
                    }
                    break;
                }
            }
        }

        selected
    }

    /// How many slots are available for dispatch?
    pub fn available_slots(&self) -> usize {
        self.capacity - self.queue.len()
    }

    /// Flush all entries.
    pub fn flush(&mut self) {
        self.queue.clear();
    }
}

/// Read a single operand value using the tag captured at rename time.
///
/// Returns `Some(value)` if the operand is ready, `None` if stalled.
fn read_operand_by_tag(
    reg: usize,
    is_fp: bool,
    tag: Option<RobTag>,
    rob: &Rob,
    cpu: &Cpu,
) -> Option<u64> {
    // x0 is hardwired zero
    if !is_fp && reg == 0 {
        return Some(0);
    }

    match tag {
        None => {
            // No in-flight producer at rename time — read from architectural register file
            Some(if is_fp {
                cpu.regs.read_f(reg)
            } else {
                cpu.regs.read(reg)
            })
        }
        Some(t) => {
            // In-flight producer — check if ROB entry has completed
            match rob.find_entry(t) {
                Some(entry) if entry.state == RobState::Completed => Some(entry.result),
                Some(_) => None, // Not ready — stall
                None => {
                    // ROB entry gone (already committed) — value is in register file
                    Some(if is_fp {
                        cpu.regs.read_f(reg)
                    } else {
                        cpu.regs.read(reg)
                    })
                }
            }
        }
    }
}
