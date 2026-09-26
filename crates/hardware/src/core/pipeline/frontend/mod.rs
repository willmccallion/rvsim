//! Frontend pipeline stages (shared across all backends).
//!
//! The frontend is generic over the execution engine and handles:
//! Fetch1 → Fetch2 → Decode → Rename
//!
//! In the event-driven design, Fetch1 emits `MemReq` packets with
//! `op = Fetch` to the L1 instruction cache and parks an `OutstandingFetch`
//! on the engine's [`BackendCommon`](crate::core::pipeline::engine::BackendCommon).
//! The pipeline-level mailbox drain pushes completed fetches into the
//! `fetch1_fetch2` latch from there. Fetch1 keeps one fetch group in
//! flight: it issues the next group only after the previous one has
//! returned and fetch2 has drained it, like gem5's fetch stage waiting in
//! `IcacheWaitResponse` and on its fetch queue. Groups inside the line the
//! I-cache last returned are served from the [`FetchBuffer`] without a new
//! request.

pub mod decode;
pub mod fetch1;
pub mod fetch2;
pub mod rename;

use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::frontend::fetch1::FetchBuffer;
use crate::core::pipeline::latches::{Fetch1Fetch2Entry, IdExEntry, IfIdEntry, RenameIssueEntry};
use crate::sim::StageCtx;
use std::marker::PhantomData;

/// The frontend pipeline, generic over the execution engine.
///
/// Same frontend code works with `InOrderEngine` and `O3Engine`.
#[derive(Debug)]
pub struct Frontend<E: ExecutionEngine> {
    /// Where fetch continues: the next PC fetch1 forms a group from. It
    /// runs ahead of the hart's architectural PC and is reset by every
    /// redirect.
    pub fetch_pc: u64,
    /// Fetch1 → Fetch2 latch (populated by the mailbox-drain stage when
    /// fetch `MemResp` packets arrive, or directly on a fetch-buffer hit).
    pub fetch1_fetch2: Vec<Fetch1Fetch2Entry>,
    /// The cache line most recently returned by the I-cache.
    pub fetch_buffer: FetchBuffer,
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
    /// Creates a new frontend with the given pipeline width, fetching from
    /// `pc`.
    pub fn new(width: usize, pc: u64) -> Self {
        Self {
            fetch_pc: pc,
            fetch1_fetch2: Vec::with_capacity(width),
            fetch_buffer: FetchBuffer::default(),
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
        state: &mut StageCtx<'_>,
        engine: &mut E,
        rename_output: &mut Vec<RenameIssueEntry>,
    ) {
        rename::rename_stage(state, &mut self.decode_rename, engine, rename_output);

        // Gate decode on rename draining to avoid O(n²) regrowth of decode_rename.
        if self.decode_rename.is_empty() && !engine.common().vector_config_unresolved {
            let vector = engine.vector_config(&state.hart().csrs);
            let ended_at_vsetvl = decode::decode_stage(
                state,
                &mut self.fetch2_decode,
                &mut self.decode_rename,
                engine.has_register_renaming(),
                vector,
            );
            engine.common_mut().vector_config_unresolved = ended_at_vsetvl;
        }

        if self.fetch2_decode.is_empty() {
            fetch2::fetch2_stage(state, &mut self.fetch1_fetch2, &mut self.fetch2_decode);
        }

        if engine.common().trap.stops_fetch() {
            return;
        }
        if engine.common().fetch_in_flight() || engine.common().fetch_held(state.cycle) {
            state.counter(state.core().stat_paths.pipeline.stalls_fetch_wait).inc();
            return;
        }
        if self.fetch1_fetch2.is_empty() {
            fetch1::fetch1_stage(
                state,
                engine,
                &mut self.fetch_buffer,
                &mut self.fetch1_fetch2,
                &mut self.fetch_pc,
            );
        }
    }

    /// True when no instruction sits in any latch between fetch and rename.
    pub const fn is_empty(&self) -> bool {
        self.fetch1_fetch2.is_empty()
            && self.fetch2_decode.is_empty()
            && self.decode_rename.is_empty()
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
