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

use crate::exec::compute::vector::mem::{is_vec_load, is_vec_store};
use crate::isa::op::VectorOp;
use crate::isa::reg::ArchReg;
use crate::trace_issue;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::backend::o3::fu_pool::{FuPool, FuType};
use crate::uarch::pipeline::backend::shared::issue_stats::IssueHold;
use crate::uarch::pipeline::backend::shared::ordering::held_by_fence;
use crate::uarch::pipeline::latches::{IssueRegisters, RenameIssueEntry, SourceReadiness};
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;
use crate::uarch::pipeline::rename::prf::PhysReg;
use crate::uarch::pipeline::rob::{HeadAtCycleStart, Rob, RobTag};
use crate::uarch::pipeline::squash::PendingSquash;

use std::collections::VecDeque;

/// The in-order backend's operand sources: the ROB entry producing each
/// of rs1, rs2 and rs3 when it was renamed, `None` to read the register
/// file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProducerTags {
    /// rs1's producer.
    pub rs1: Option<RobTag>,
    /// rs2's producer.
    pub rs2: Option<RobTag>,
    /// rs3's producer.
    pub rs3: Option<RobTag>,
}

impl IssueRegisters for ProducerTags {
    /// x0's: the in-order backend renames nothing, so writeback ignores it.
    fn rd_phys(&self) -> PhysReg {
        PhysReg(0)
    }

    fn readiness(&self) -> SourceReadiness {
        SourceReadiness { rs1: self.rs1.is_none(), rs2: self.rs2.is_none() }
    }
}

/// An instruction the in-order backend renamed.
pub type InOrderEntry = RenameIssueEntry<ProducerTags>;

/// The unit an issued instruction took and when its result is ready.
#[derive(Clone, Copy, Debug)]
pub struct IssuedUnit {
    /// The instruction.
    pub tag: RobTag,
    /// The unit it executes on.
    pub fu_type: FuType,
    /// Cycle the unit delivers the result.
    pub complete_cycle: u64,
    /// Cycles the unit could take nothing else.
    pub busy_cycles: u64,
}

/// What the in-order issue stage reads to decide what may issue.
#[derive(Clone, Copy, Debug)]
pub struct IssueContext<'a> {
    /// The instructions in flight, whose results bypass to their consumers.
    pub rob: &'a Rob,
    /// The oldest instruction as the cycle began.
    pub head: HeadAtCycleStart,
    /// The stores whose addresses a load must not pass while unresolved.
    pub store_buffer: &'a StoreBuffer,
    /// The vector stores, likewise.
    pub vec_store_buffer: &'a VecStoreBuffer,
    /// The squash a resolved misprediction has asked for.
    pub pending_squash: Option<PendingSquash>,
    /// The current cycle.
    pub now: u64,
}

/// What issue selected this cycle.
#[derive(Debug, Default)]
pub struct Selection {
    /// The instructions issued, with their operands read, oldest first.
    pub issued: Vec<InOrderEntry>,
    /// The unit each took and when its result is ready.
    pub units: Vec<IssuedUnit>,
    /// What held the head of the queue, when something did.
    pub held: Option<IssueHold>,
}

/// FIFO issue unit for in-order execution.
#[derive(Debug)]
pub struct InOrderIssueUnit {
    queue: VecDeque<InOrderEntry>,
    capacity: usize,
}

impl InOrderIssueUnit {
    /// Creates a new FIFO issue unit with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self { queue: VecDeque::with_capacity(capacity), capacity }
    }

    /// Accept dispatched instructions from rename, which takes no more
    /// than [`Self::available_slots`] a cycle, so the queue never overflows.
    pub fn dispatch(&mut self, entries: Vec<InOrderEntry>) {
        for entry in entries {
            debug_assert!(
                self.queue.len() < self.capacity,
                "issue queue overflow: len={} capacity={}, rob_tag={} pc={:#x}",
                self.queue.len(),
                self.capacity,
                entry.rob_tag,
                entry.inst.pc,
            );
            self.queue.push_back(entry);
        }
    }

    /// Select instructions to execute this cycle, reading operands via
    /// tags captured at rename time, and taking a unit for each; a trapped
    /// instruction takes none. Up to `width` instructions issue.
    ///
    /// In-order: if the head-of-queue is blocked, nothing behind it can
    /// issue, and the selection says what held it. Nothing a pending squash
    /// will remove issues at all: an in-order core drops the instructions
    /// behind a resolved misprediction at once.
    pub fn select(
        &mut self,
        width: usize,
        ctx: &IssueContext<'_>,
        state: &StageCtx<'_>,
        fu_pool: &mut FuPool,
    ) -> Selection {
        let mut selection = Selection {
            issued: Vec::with_capacity(width),
            units: Vec::with_capacity(width),
            held: None,
        };
        for _ in 0..width {
            let Some(entry) = self.queue.front() else { break };
            if ctx.pending_squash.is_some_and(|squash| squash.squashes(entry.rob_tag)) {
                selection.held = Some(IssueHold::Ordering);
                break;
            }
            if entry.trap.is_some() {
                selection.issued.extend(self.queue.pop_front());
                continue;
            }
            if is_held_by_ordering(entry, ctx) {
                selection.held = Some(IssueHold::Ordering);
                break;
            }
            let Some(operands) = read_operands(entry, ctx.rob, state) else {
                selection.held = Some(IssueHold::Operands);
                break;
            };
            let Some(unit) = take_unit(fu_pool, entry, state, ctx.now) else {
                selection.held = Some(IssueHold::Unit);
                break;
            };
            let Some(mut issued) = self.queue.pop_front() else { break };
            [issued.inst.rv1, issued.inst.rv2, issued.inst.rv3] = operands;
            selection.units.push(unit);
            selection.issued.push(issued);
        }
        selection
    }

    /// Return a snapshot of the current issue queue contents (front = oldest).
    pub fn queue_snapshot(&self) -> Vec<RenameIssueEntry<SourceReadiness>> {
        self.queue.iter().map(RenameIssueEntry::snapshot).collect()
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

/// Whether ordering holds `entry` back: a vector op, or an atomic that
/// takes effect in the cache, executes only as the oldest instruction, as
/// a vector op reads the architectural vector registers; a fence waits for
/// what it orders; and a load waits for older store addresses, or
/// forwarding could miss an overlap. A system instruction flows: its
/// effect waits for commit.
fn is_held_by_ordering(entry: &InOrderEntry, ctx: &IssueContext<'_>) -> bool {
    let ctrl = &entry.inst.ctrl;
    let tag = entry.rob_tag;
    let waits_for_head =
        (ctrl.vec_op != VectorOp::None && !ctrl.vec_op.is_config()) || ctrl.performs_at_rob_head();
    (waits_for_head && !ctx.head.is(tag))
        || held_by_fence(ctx.rob, tag, &entry.inst)
        || (ctrl.mem_read
            && (ctx.store_buffer.has_unresolved_store_before(tag)
                || ctx.vec_store_buffer.has_unresolved_store_before(tag)))
}

/// `entry`'s three source operands, or `None`, traced, while one is still
/// being computed.
fn read_operands(entry: &InOrderEntry, rob: &Rob, state: &StageCtx<'_>) -> Option<[u64; 3]> {
    let ctrl = &entry.inst.ctrl;
    let tags = &entry.regs;
    let rv1 = read_operand_by_tag(ctrl.src1(entry.inst.rs1), tags.rs1, rob, state);
    let rv2 = read_operand_by_tag(ctrl.src2(entry.inst.rs2), tags.rs2, rob, state);
    let rv3 = ctrl
        .src3(entry.inst.rs3)
        .map_or(Some(0), |rs3| read_operand_by_tag(rs3, tags.rs3, rob, state));
    if let (Some(v1), Some(v2), Some(v3)) = (rv1, rv2, rv3) {
        return Some([v1, v2, v3]);
    }
    trace_issue!(state.config.general.trace_instructions;
        pc       = %crate::common::trace::Hex(entry.inst.pc.val()),
        rs1      = entry.inst.rs1.as_usize(),
        rs1_tag  = ?entry.regs.rs1,
        rs1_rdy  = rv1.is_some(),
        rs2      = entry.inst.rs2.as_usize(),
        rs2_tag  = ?entry.regs.rs2,
        rs2_rdy  = rv2.is_some(),
        "IS: stall — operand not ready"
    );
    None
}

/// Takes a free unit of `entry`'s kind, or `None` while every one is busy.
fn take_unit(
    fu_pool: &mut FuPool,
    entry: &InOrderEntry,
    state: &StageCtx<'_>,
    now: u64,
) -> Option<IssuedUnit> {
    let fu_type = FuType::classify(&entry.inst.ctrl);
    let unit = fu_pool.free_unit(fu_type, now)?;
    let complete_cycle = if is_vector_arithmetic(entry.inst.ctrl.vec_op) {
        // A vector op issues only as the oldest instruction, so the
        // architectural vl is the one it executes under.
        let latency = fu_pool.vector_op_latency(
            fu_type,
            &entry.inst.ctrl,
            state.hart().csrs.vl as usize,
            state.config.pipeline.vector_lanes(),
        );
        fu_pool.acquire_with_latency(unit, now, latency, entry.rob_tag)
    } else {
        fu_pool.acquire(unit, now, entry.rob_tag)
    };
    let busy_cycles = fu_pool.busy_cycles(unit, now);
    Some(IssuedUnit { tag: entry.rob_tag, fu_type, complete_cycle, busy_cycles })
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
    reg: ArchReg,
    tag: Option<RobTag>,
    rob: &Rob,
    state: &StageCtx<'_>,
) -> Option<u64> {
    if reg.is_hardwired_zero() {
        return Some(0);
    }

    let from_register_file = || {
        Some(match reg {
            ArchReg::Int(idx) => state.hart().regs.read(idx),
            ArchReg::Fp(idx) => state.hart().regs.read_f(idx),
        })
    };
    let Some(tag) = tag else { return from_register_file() };
    // A producer no longer in the ROB has committed its value to the register file.
    rob.find_entry(tag).map_or_else(from_register_file, |entry| entry.result)
}
