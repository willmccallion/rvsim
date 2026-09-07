//! Frontend pipeline stages (shared across all backends).
//!
//! The frontend is generic over the execution engine and handles:
//! Fetch1 → Fetch2 → Decode → Rename
//!
//! In the event-driven design, Fetch1 emits `MemReq` packets with
//! `op = Fetch` to the L1 instruction cache and parks an `OutstandingFetch`
//! on the engine's [`BackendCommon`](crate::core::pipeline::engine::BackendCommon).
//! The pipeline-level mailbox drain pushes completed fetches into the
//! `fetch1_fetch2` latch from there.

pub mod decode;
pub mod fetch1;
pub mod fetch2;
pub mod rename;

use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::latches::{Fetch1Fetch2Entry, IdExEntry, IfIdEntry, RenameIssueEntry};
use std::marker::PhantomData;

/// The frontend pipeline, generic over the execution engine.
///
/// Same frontend code works with `InOrderEngine` and `O3Engine`.
#[derive(Debug)]
pub struct Frontend<E: ExecutionEngine> {
    /// Fetch1 → Fetch2 latch (populated by the mailbox-drain stage when
    /// fetch `MemResp` packets arrive).
    pub fetch1_fetch2: Vec<Fetch1Fetch2Entry>,
    /// Fetch2 → Decode latch.
    pub fetch2_decode: Vec<IfIdEntry>,
    /// Decode → Rename latch.
    pub decode_rename: Vec<IdExEntry>,
    /// Retained for snapshot compatibility; the packet model no longer
    /// uses it (fetch latency arrives through `MemResp` arrival cycle).
    pub fetch1_stall: u64,
    /// Retained for snapshot compatibility; the packet model no longer
    /// uses it (cache-miss penalty arrives through `MemResp` arrival cycle).
    pub fetch2_stall: u64,
    _marker: PhantomData<E>,
}

impl<E: ExecutionEngine> Frontend<E> {
    /// Creates a new frontend with the given pipeline width.
    pub fn new(width: usize) -> Self {
        Self {
            fetch1_fetch2: Vec::with_capacity(width),
            fetch2_decode: Vec::with_capacity(width),
            decode_rename: Vec::with_capacity(width),
            fetch1_stall: 0,
            fetch2_stall: 0,
            _marker: PhantomData,
        }
    }

    /// Executes one cycle of all frontend stages (reverse order).
    pub fn tick(
        &mut self,
        state: &mut crate::sim::SimState,
        engine: &mut E,
        rename_output: &mut Vec<RenameIssueEntry>,
    ) {
        rename::rename_stage(state, &mut self.decode_rename, engine, rename_output);

        // Gate decode on rename draining to avoid O(n²) regrowth of decode_rename.
        if self.decode_rename.is_empty() {
            decode::decode_stage(
                state,
                &mut self.fetch2_decode,
                &mut self.decode_rename,
                engine.has_register_renaming(),
            );
        }

        if self.fetch2_decode.is_empty() {
            fetch2::fetch2_stage(state, &mut self.fetch1_fetch2, &mut self.fetch2_decode);
        }

        // Fetch1 always emits — it parks every fetch in the engine's
        // outstanding_fetches table. The mailbox-drain stage at the top
        // of the next pipeline tick (after responses arrive) pushes the
        // matching Fetch1Fetch2Entry into `fetch1_fetch2`.
        fetch1::fetch1_stage(state, engine);
    }

    /// Flushes all frontend latches.
    pub fn flush(&mut self) {
        self.fetch1_fetch2.clear();
        self.fetch2_decode.clear();
        self.decode_rename.clear();
        self.fetch1_stall = 0;
        self.fetch2_stall = 0;
    }
}
