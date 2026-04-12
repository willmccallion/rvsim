//! In-Order Issue Unit: FIFO queue with tag-based operand read.
//!
//! For the in-order backend, issue is a FIFO queue. When selecting instructions,
//! the issue stage reads operand values using the tags captured at rename time:
//! - If tag is None → read from architectural register file.
//! - If tag points to a completed ROB entry → bypass the result.
//! - If the ROB entry is still in-flight → stall (operand not ready).

use crate::common::RegIdx;
use crate::core::Cpu;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::rob::{Rob, RobState, RobTag};
use crate::core::pipeline::signals::SystemOp;
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::trace_issue;

use std::collections::VecDeque;

/// FIFO issue unit for in-order execution.
#[derive(Debug)]
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
        Self { queue: VecDeque::with_capacity(capacity), capacity }
    }

    /// Accept dispatched instructions from rename.
    pub fn dispatch(&mut self, entries: Vec<RenameIssueEntry>) {
        for entry in entries {
            debug_assert!(
                self.queue.len() < self.capacity,
                "issue queue overflow: len={} capacity={} — entry rob_tag={} pc={:#x} would be silently dropped",
                self.queue.len(),
                self.capacity,
                entry.rob_tag.0,
                entry.pc,
            );
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
    pub fn select(
        &mut self,
        width: usize,
        rob: &Rob,
        store_buffer: &StoreBuffer,
        cpu: &Cpu,
    ) -> Vec<RenameIssueEntry> {
        let mut selected = Vec::with_capacity(width);

        for _ in 0..width {
            let Some(entry) = self.queue.front() else { break };

            // Faulted instructions don't need operands — pass through
            if entry.trap.is_some() {
                if let Some(e) = self.queue.pop_front() {
                    selected.push(e);
                }
                continue;
            }

            // ── Serialization checks (matching O3 issue queue) ──────────

            // System/CSR instructions are serializing: wait for all older
            // instructions to complete before issuing.
            if entry.ctrl.system_op != SystemOp::None && !rob.all_before_completed(entry.rob_tag) {
                break;
            }

            // FENCE: wait for older operations matching pred bits to complete.
            if entry.ctrl.system_op == SystemOp::Fence {
                let pred_bits = ((entry.inst >> 24) & 0xF) as u8;
                let pred_r = pred_bits & 0b0010 != 0;
                let pred_w = pred_bits & 0b0001 != 0;
                if !rob.fence_pred_satisfied(entry.rob_tag, pred_r, pred_w) {
                    break;
                }
            }

            // Loads/stores: blocked by older in-flight FENCE with matching succ bits.
            if (entry.ctrl.mem_read || entry.ctrl.mem_write)
                && rob.has_fence_blocking(entry.rob_tag, entry.ctrl.mem_read, entry.ctrl.mem_write)
            {
                break;
            }

            // Loads must wait for all older stores to have resolved addresses,
            // otherwise store-to-load forwarding can miss an overlap.
            if entry.ctrl.mem_read && store_buffer.has_unresolved_store_before(entry.rob_tag) {
                break;
            }

            // Vector memory ops access memory directly via the bus (in-order
            // backend), bypassing the store buffer. They must wait for ALL
            // older instructions to complete so that memory state is consistent.
            if (crate::core::units::vpu::mem::is_vec_load(entry.ctrl.vec_op)
                || crate::core::units::vpu::mem::is_vec_store(entry.ctrl.vec_op))
                && !rob.all_before_completed(entry.rob_tag)
            {
                break;
            }

            // ── Operand readiness ───────────────────────────────────────

            // Try to read all source operands using tags captured at rename
            let rv1 = read_operand_by_tag(entry.rs1, entry.ctrl.rs1_fp, entry.rs1_tag, rob, cpu);
            let rv2 = read_operand_by_tag(entry.rs2, entry.ctrl.rs2_fp, entry.rs2_tag, rob, cpu);
            let rv3 = if entry.ctrl.rs3_fp {
                read_operand_by_tag(entry.rs3, true, entry.rs3_tag, rob, cpu)
            } else {
                Some(0)
            };

            if let (Some(v1), Some(v2), Some(v3)) = (rv1, rv2, rv3) {
                let Some(mut issued) = self.queue.pop_front() else { break };
                issued.rv1 = v1;
                issued.rv2 = v2;
                issued.rv3 = v3;
                selected.push(issued);
            } else {
                // Head of queue blocked — in-order can't skip
                trace_issue!(cpu.trace;
                    pc       = %crate::trace::Hex(entry.pc),
                    rs1      = entry.rs1.as_usize(),
                    rs1_tag  = ?entry.rs1_tag,
                    rs1_rdy  = rv1.is_some(),
                    rs2      = entry.rs2.as_usize(),
                    rs2_tag  = ?entry.rs2_tag,
                    rs2_rdy  = rv2.is_some(),
                    "IS: stall — operand not ready"
                );
                break;
            }
        }

        selected
    }

    /// Return a snapshot of the current issue queue contents (front = oldest).
    pub fn queue_snapshot(&self) -> Vec<RenameIssueEntry> {
        self.queue.iter().cloned().collect()
    }

    /// How many slots are available for dispatch?
    pub fn available_slots(&self) -> usize {
        self.capacity - self.queue.len()
    }

    /// How many instructions are in the issue queue?
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the issue queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
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
    reg: RegIdx,
    is_fp: bool,
    tag: Option<RobTag>,
    rob: &Rob,
    cpu: &Cpu,
) -> Option<u64> {
    // x0 is hardwired zero
    if !is_fp && reg.is_zero() {
        return Some(0);
    }

    tag.map_or_else(
        || {
            // No in-flight producer at rename time — read from architectural register file
            let val = if is_fp { cpu.regs.read_f(reg) } else { cpu.regs.read(reg) };
            Some(val)
        },
        |t| {
            // In-flight producer — check if ROB entry has completed
            match rob.find_entry(t) {
                Some(entry) if entry.state == RobState::Completed => entry.result,
                Some(_) => None, // Not ready — stall
                None => {
                    // ROB entry gone (already committed) — value is in register file
                    Some(if is_fp { cpu.regs.read_f(reg) } else { cpu.regs.read(reg) })
                }
            }
        },
    )
}
