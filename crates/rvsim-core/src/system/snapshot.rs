//! A point-in-time copy of what every pipeline stage holds, as plain data.
//!
//! [`Simulator::pipeline_snapshot`](crate::system::simulator::Simulator::pipeline_snapshot)
//! copies the inter-stage latches after a tick so a host can inspect what
//! is in flight without a borrow on the live system.

use crate::common::{PhysAddr, VirtAddr};
use crate::isa::reg::RegIdx;
use crate::uarch::pipeline::snapshot::LatchSnapshot;

/// What every inter-stage latch of one core's pipeline holds.
///
/// Stages in order: Fetch1, Fetch2, Decode, Rename, Issue, Execute, Mem1,
/// Mem2, Writeback, Commit. Each stage holds at most `width` slots; an
/// empty stage is stalled or idle this cycle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PipelineSnapshot {
    /// The pipeline width.
    pub width: usize,
    /// Fetch1 to Fetch2: PCs chosen, awaiting the instruction cache.
    pub fetch1_fetch2: Vec<FetchSlot>,
    /// Fetch2 to Decode: instruction words fetched.
    pub fetch2_decode: Vec<FetchedSlot>,
    /// Decode to Rename: decoded instructions.
    pub decode_rename: Vec<DecodedSlot>,
    /// Rename to Issue: renamed instructions waiting to dispatch.
    pub rename_issue: Vec<RenamedSlot>,
    /// The issue queue, oldest first.
    pub issue_queue: Vec<RenamedSlot>,
    /// Execute to Mem1.
    pub execute_mem1: Vec<ExecutedSlot>,
    /// Mem1 to Mem2.
    pub mem1_mem2: Vec<MemorySlot>,
    /// Mem2 to Writeback.
    pub mem2_wb: Vec<WritebackSlot>,
}

/// A PC the frontend chose to fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchSlot {
    /// The address to fetch.
    pub pc: u64,
    /// Whether the predictor said the instruction there is a taken branch.
    pub pred_taken: bool,
    /// The predicted target, when taken.
    pub pred_target: u64,
}

/// An instruction word fetched, before decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FetchedSlot {
    /// Its address.
    pub pc: u64,
    /// Its encoding.
    pub raw: u32,
    /// Whether the predictor said it is a taken branch.
    pub pred_taken: bool,
    /// The predicted target, when taken.
    pub pred_target: u64,
}

/// A decoded instruction with its operands read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedSlot {
    /// Its address.
    pub pc: u64,
    /// Its encoding.
    pub raw: u32,
    /// First source register.
    pub rs1: RegIdx,
    /// Second source register.
    pub rs2: RegIdx,
    /// Destination register.
    pub rd: RegIdx,
    /// The immediate.
    pub imm: i64,
    /// The value read for `rs1`.
    pub rv1: u64,
    /// The value read for `rs2`.
    pub rv2: u64,
}

/// A renamed instruction holding a reorder-buffer slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenamedSlot {
    /// Its address.
    pub pc: u64,
    /// Its encoding.
    pub raw: u32,
    /// First source register.
    pub rs1: RegIdx,
    /// Second source register.
    pub rs2: RegIdx,
    /// Destination register.
    pub rd: RegIdx,
    /// The value read for `rs1`.
    pub rv1: u64,
    /// The value read for `rs2`.
    pub rv2: u64,
    /// Its reorder-buffer tag.
    pub rob_tag: u32,
    /// Whether its first operand is available.
    pub rs1_ready: bool,
    /// Whether its second operand is available.
    pub rs2_ready: bool,
}

/// An executed instruction carrying its result to the memory stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutedSlot {
    /// Its address.
    pub pc: u64,
    /// Its encoding.
    pub raw: u32,
    /// Destination register.
    pub rd: RegIdx,
    /// The ALU result, available to dependents by forwarding.
    pub alu: u64,
    /// The data a store writes.
    pub store_data: u64,
    /// Its reorder-buffer tag.
    pub rob_tag: u32,
}

/// An instruction in the memory stages with its address translated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemorySlot {
    /// Its address.
    pub pc: u64,
    /// Its encoding.
    pub raw: u32,
    /// Destination register.
    pub rd: RegIdx,
    /// The ALU result.
    pub alu: u64,
    /// The virtual address it accesses.
    pub vaddr: VirtAddr,
    /// The physical address it accesses.
    pub paddr: PhysAddr,
    /// The data a store writes.
    pub store_data: u64,
    /// Its reorder-buffer tag.
    pub rob_tag: u32,
}

/// An instruction about to write back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WritebackSlot {
    /// Its address.
    pub pc: u64,
    /// Its encoding.
    pub raw: u32,
    /// Destination register.
    pub rd: RegIdx,
    /// The ALU result.
    pub alu: u64,
    /// The data a load returned.
    pub load_data: u64,
    /// Its reorder-buffer tag.
    pub rob_tag: u32,
}

impl From<&LatchSnapshot> for PipelineSnapshot {
    fn from(latches: &LatchSnapshot) -> Self {
        Self {
            width: latches.width,
            fetch1_fetch2: latches
                .fetch1_fetch2
                .iter()
                .map(|e| FetchSlot {
                    pc: e.pc,
                    pred_taken: e.pred_taken,
                    pred_target: e.pred_target,
                })
                .collect(),
            fetch2_decode: latches
                .fetch2_decode
                .iter()
                .map(|e| FetchedSlot {
                    pc: e.pc,
                    raw: e.inst,
                    pred_taken: e.pred_taken,
                    pred_target: e.pred_target,
                })
                .collect(),
            decode_rename: latches
                .decode_rename
                .iter()
                .map(|e| DecodedSlot {
                    pc: e.inst.pc,
                    raw: e.inst.bits,
                    rs1: e.inst.rs1,
                    rs2: e.inst.rs2,
                    rd: e.inst.rd,
                    imm: e.inst.imm,
                    rv1: e.inst.rv1,
                    rv2: e.inst.rv2,
                })
                .collect(),
            rename_issue: latches.rename_issue.iter().map(renamed_slot).collect(),
            issue_queue: latches.issue_queue.iter().map(renamed_slot).collect(),
            execute_mem1: latches
                .execute_mem1
                .iter()
                .map(|e| ExecutedSlot {
                    pc: e.pc,
                    raw: e.inst,
                    rd: e.rd,
                    alu: e.alu,
                    store_data: e.store_data,
                    rob_tag: e.rob_tag.0,
                })
                .collect(),
            mem1_mem2: latches
                .mem1_mem2
                .iter()
                .map(|e| MemorySlot {
                    pc: e.pc,
                    raw: e.inst,
                    rd: e.rd,
                    alu: e.alu,
                    vaddr: e.vaddr,
                    paddr: e.paddr,
                    store_data: e.store_data,
                    rob_tag: e.rob_tag.0,
                })
                .collect(),
            mem2_wb: latches
                .mem2_wb
                .iter()
                .map(|e| WritebackSlot {
                    pc: e.pc,
                    raw: e.inst,
                    rd: e.rd,
                    alu: e.alu,
                    load_data: e.load_data,
                    rob_tag: e.rob_tag.0,
                })
                .collect(),
        }
    }
}

const fn renamed_slot(e: &crate::uarch::pipeline::latches::RenameIssueEntry) -> RenamedSlot {
    RenamedSlot {
        pc: e.inst.pc,
        raw: e.inst.bits,
        rs1: e.inst.rs1,
        rs2: e.inst.rs2,
        rd: e.inst.rd,
        rv1: e.inst.rv1,
        rv2: e.inst.rv2,
        rob_tag: e.rob_tag.0,
        rs1_ready: e.rs1_tag.is_none(),
        rs2_ready: e.rs2_tag.is_none(),
    }
}
