//! In-Order Issue Unit: FIFO queue with tag-based operand read.
//!
//! For the in-order backend, issue is a FIFO queue. When selecting instructions,
//! the issue stage reads operand values using the tags captured at rename time:
//! - If tag is None → read from architectural register file.
//! - If the producing ROB entry has a result (forwarded from its unit or
//!   written back) → bypass the result.
//! - If the producer is still executing → stall (operand not ready).
//!
//! An instruction also needs a free functional unit of its kind; it takes
//! the unit when it issues, and the unit reports when the result is ready.

use crate::common::RegIdx;
use crate::core::exec::signals::{SystemOp, VectorOp};
use crate::core::pipeline::backend::o3::fu_pool::{FuPool, FuType};
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::rob::{Rob, RobTag};
use crate::core::pipeline::squash::PendingSquash;
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::core::pipeline::vec_store_buffer::VecStoreBuffer;
use crate::core::units::vpu::mem::{is_vec_load, is_vec_store};
use crate::sim::StageCtx;
use crate::trace_issue;

use std::collections::VecDeque;

/// The unit an issued instruction took and when its result is ready.
#[derive(Clone, Copy, Debug)]
pub struct IssuedUnit {
    /// The instruction.
    pub tag: RobTag,
    /// The unit it executes on.
    pub fu_type: FuType,
    /// Cycle the unit delivers the result.
    pub complete_cycle: u64,
}

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
                entry.inst.pc,
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
    /// Nothing a pending squash will remove issues at all: an in-order
    /// core drops the instructions behind a resolved misprediction at once.
    ///
    /// Each issued instruction's unit and the cycle its result is ready are
    /// returned alongside it; a trapped instruction takes no unit.
    #[allow(clippy::too_many_arguments)]
    pub fn select(
        &mut self,
        width: usize,
        rob: &Rob,
        store_buffer: &StoreBuffer,
        vec_store_buffer: &VecStoreBuffer,
        state: &mut StageCtx<'_>,
        fu_pool: &mut FuPool,
        now: u64,
        pending_squash: Option<PendingSquash>,
    ) -> (Vec<RenameIssueEntry>, Vec<IssuedUnit>) {
        let mut selected = Vec::with_capacity(width);
        let mut units = Vec::with_capacity(width);

        for _ in 0..width {
            let Some(entry) = self.queue.front() else { break };
            if pending_squash.is_some_and(|squash| squash.squashes(entry.rob_tag)) {
                break;
            }

            if entry.trap.is_some() {
                if let Some(e) = self.queue.pop_front() {
                    selected.push(e);
                }
                continue;
            }

            // A system instruction reads or writes architectural state, so
            // it executes only as the oldest instruction; FENCE and the CBOs
            // have their own checks below. A vector instruction reads the
            // architectural vector registers, so it waits for the head too,
            // and so does an atomic that takes effect in the cache.
            let waits_for_head = (entry.inst.ctrl.vec_op != VectorOp::None
                && !entry.inst.ctrl.vec_op.is_config())
                || (entry.inst.ctrl.system_op != SystemOp::None
                    && entry.inst.ctrl.system_op != SystemOp::Fence
                    && !entry.inst.ctrl.system_op.is_cbo())
                || entry.inst.ctrl.performs_at_rob_head();
            if waits_for_head && !rob.is_head(entry.rob_tag) {
                break;
            }

            if entry.inst.ctrl.system_op == SystemOp::Fence {
                let pred_bits = ((entry.inst.bits >> 24) & 0xF) as u8;
                let pred_r = pred_bits & 0b0010 != 0;
                let pred_w = pred_bits & 0b0001 != 0;
                if !rob.fence_pred_satisfied(entry.rob_tag, pred_r, pred_w) {
                    break;
                }
            }

            let (reads, writes) = (entry.inst.ctrl.reads_memory(), entry.inst.ctrl.writes_memory());
            if (reads || writes) && rob.has_fence_blocking(entry.rob_tag, reads, writes) {
                break;
            }

            // Loads need older store addresses resolved or forwarding can miss an overlap.
            if entry.inst.ctrl.mem_read
                && (store_buffer.has_unresolved_store_before(entry.rob_tag)
                    || vec_store_buffer.has_unresolved_store_before(entry.rob_tag))
            {
                break;
            }

            let rv1 = read_operand_by_tag(
                entry.inst.rs1,
                entry.inst.ctrl.rs1_fp,
                entry.rs1_tag,
                rob,
                state,
            );
            let rv2 = read_operand_by_tag(
                entry.inst.rs2,
                entry.inst.ctrl.rs2_fp,
                entry.rs2_tag,
                rob,
                state,
            );
            let rv3 = if entry.inst.ctrl.rs3_fp {
                read_operand_by_tag(entry.inst.rs3, true, entry.rs3_tag, rob, state)
            } else {
                Some(0)
            };

            if let (Some(v1), Some(v2), Some(v3)) = (rv1, rv2, rv3) {
                let fu_type = FuType::classify(&entry.inst.ctrl);
                let Some(unit) = fu_pool.free_unit(fu_type, now) else {
                    state.counter(state.core().stat_paths.pipeline.stalls_fu_structural).inc();
                    break;
                };
                let complete_cycle = if is_vector_arithmetic(entry.inst.ctrl.vec_op) {
                    // A vector op issues only as the oldest instruction, so
                    // the architectural vl is the one it executes under.
                    let latency = fu_pool.vector_op_latency(
                        fu_type,
                        &entry.inst.ctrl,
                        state.hart().csrs.vl as usize,
                        state.config.pipeline.vector_lanes(),
                    );
                    fu_pool.acquire_with_latency(unit, now, latency)
                } else {
                    fu_pool.acquire(unit, now)
                };
                let Some(mut issued) = self.queue.pop_front() else { break };
                units.push(IssuedUnit { tag: issued.rob_tag, fu_type, complete_cycle });
                issued.inst.rv1 = v1;
                issued.inst.rv2 = v2;
                issued.inst.rv3 = v3;
                selected.push(issued);
            } else {
                trace_issue!(state.config.general.trace_instructions;
                    pc       = %crate::trace::Hex(entry.inst.pc),
                    rs1      = entry.inst.rs1.as_usize(),
                    rs1_tag  = ?entry.rs1_tag,
                    rs1_rdy  = rv1.is_some(),
                    rs2      = entry.inst.rs2.as_usize(),
                    rs2_tag  = ?entry.rs2_tag,
                    rs2_rdy  = rv2.is_some(),
                    "IS: stall — operand not ready"
                );
                break;
            }
        }

        (selected, units)
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

/// A vector op that computes in a vector unit rather than setting the
/// configuration or moving memory.
const fn is_vector_arithmetic(op: VectorOp) -> bool {
    !matches!(op, VectorOp::None) && !op.is_config() && !is_vec_load(op) && !is_vec_store(op)
}

/// Read a single operand value using the tag captured at rename time.
///
/// Returns `Some(value)` if the operand is ready, `None` if stalled.
fn read_operand_by_tag(
    reg: RegIdx,
    is_fp: bool,
    tag: Option<RobTag>,
    rob: &Rob,
    state: &StageCtx<'_>,
) -> Option<u64> {
    if !is_fp && reg.is_zero() {
        return Some(0);
    }

    let from_register_file =
        || Some(if is_fp { state.hart().regs.read_f(reg) } else { state.hart().regs.read(reg) });
    let Some(tag) = tag else { return from_register_file() };
    // A producer no longer in the ROB has committed its value to the register file.
    rob.find_entry(tag).map_or_else(from_register_file, |entry| entry.result)
}
