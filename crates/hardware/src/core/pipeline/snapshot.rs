//! Pipeline snapshot: a point-in-time read-only copy of all inter-stage latches.
//!
//! Calling `PipelineDispatch::snapshot()` clones the latch vectors so Python can
//! inspect the full pipeline state after any `tick()` without holding a borrow
//! on the live simulator.

use crate::core::pipeline::latches::{
    ExMem1Entry, Fetch1Fetch2Entry, IdExEntry, IfIdEntry, Mem1Mem2Entry, Mem2WbEntry,
    RenameIssueEntry,
};

/// Point-in-time copy of every inter-stage latch in the pipeline.
///
/// Stages in order:
///   Fetch1 → Fetch2 → Decode → Rename → Issue → Execute → Mem1 → Mem2 → Writeback → Commit.
/// Each field is a `Vec` of latch entries; the length is at most `pipeline_width`.
/// An empty vec means the stage is stalled or idle this cycle.
#[derive(Clone, Debug, Default)]
pub struct PipelineSnapshot {
    /// Fetch1 → Fetch2 latch (PC gen / I-TLB).
    pub fetch1_fetch2: Vec<Fetch1Fetch2Entry>,
    /// Fetch2 → Decode latch (I-cache access).
    pub fetch2_decode: Vec<IfIdEntry>,
    /// Decode → Rename latch (instruction decode).
    pub decode_rename: Vec<IdExEntry>,
    /// Rename → Issue latch (register rename / ROB allocation, pending dispatch).
    pub rename_issue: Vec<RenameIssueEntry>,
    /// Issue queue contents (waiting for operands, front = oldest).
    pub issue_queue: Vec<RenameIssueEntry>,
    /// Execute → Memory1 latch.
    pub execute_mem1: Vec<ExMem1Entry>,
    /// Memory1 → Memory2 latch.
    pub mem1_mem2: Vec<Mem1Mem2Entry>,
    /// Memory2 → Writeback latch.
    pub mem2_wb: Vec<Mem2WbEntry>,
    /// Number of active frontend fetch1 stall cycles remaining.
    pub fetch1_stall: u64,
    /// Number of active frontend fetch2 stall cycles remaining.
    pub fetch2_stall: u64,
    /// Number of active memory1 stall cycles remaining.
    pub mem1_stall: u64,
    /// Pipeline width (superscalar degree).
    pub width: usize,
}
