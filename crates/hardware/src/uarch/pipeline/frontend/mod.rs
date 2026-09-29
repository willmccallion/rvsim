//! Frontend pipeline stages (shared across all backends).
//!
//! The frontend is generic over the execution engine and handles:
//! Fetch1 → Fetch2 → Decode → Rename
//!
//! In the event-driven design, Fetch1 emits `MemReq` packets with
//! `op = Fetch` to the L1 instruction cache and parks an `OutstandingFetch`
//! on the engine's [`BackendCommon`](crate::uarch::pipeline::engine::BackendCommon).
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

use crate::system::StageCtx;
use crate::uarch::pipeline::engine::ExecutionEngine;
use crate::uarch::pipeline::frontend::fetch1::FetchBuffer;
use crate::uarch::pipeline::latches::{
    Fetch1Fetch2Entry, IdExEntry, IfIdEntry, Latch, RenameIssueEntry,
};
use std::marker::PhantomData;

/// Cycles from a stage writing its latch to the next stage reading it.
pub const STAGE_DELAY: u64 = 1;

/// The fetch1→fetch2 latch is the I-cache's landing point: its response
/// already carried the access latency, so fetch2 reads it the same cycle.
const FETCH_LANDING_DELAY: u64 = 0;

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
    pub fetch1_fetch2: Latch<Fetch1Fetch2Entry>,
    /// The cache line most recently returned by the I-cache.
    pub fetch_buffer: FetchBuffer,
    /// Fetch2 → Decode latch.
    pub fetch2_decode: Latch<IfIdEntry>,
    /// Decode → Rename latch.
    pub decode_rename: Latch<IdExEntry>,
    _marker: PhantomData<E>,
}

impl<E: ExecutionEngine> Frontend<E> {
    /// Creates a new frontend fetching from `pc`.
    pub fn new(pc: u64) -> Self {
        Self {
            fetch_pc: pc,
            fetch1_fetch2: Latch::new(FETCH_LANDING_DELAY),
            fetch_buffer: FetchBuffer::default(),
            fetch2_decode: Latch::new(STAGE_DELAY),
            decode_rename: Latch::new(STAGE_DELAY),
            _marker: PhantomData,
        }
    }

    /// Executes one cycle of all frontend stages (reverse order). Each
    /// stage runs only when the latch it writes is empty, which is how a
    /// stall propagates back to fetch.
    pub fn tick(
        &mut self,
        state: &mut StageCtx<'_>,
        engine: &mut E,
        rename_output: &mut Latch<RenameIssueEntry>,
    ) {
        let now = state.cycle;

        if let Some(decoded) = self.decode_rename.ready(now) {
            let mut renamed = Vec::new();
            rename::rename_stage(state, decoded, engine, &mut renamed);
            rename_output.push(now, renamed);
        }

        if self.decode_rename.is_empty()
            && !engine.common().vector_config_unresolved
            && let Some(fetched) = self.fetch2_decode.ready(now)
        {
            let vector = engine.vector_config(&state.hart().csrs);
            let mut decoded = Vec::new();
            let outcome = decode::decode_stage(
                state,
                fetched,
                &mut decoded,
                engine.has_register_renaming(),
                vector,
            );
            engine.common_mut().vector_config_unresolved = outcome.ended_at_vsetvl;
            self.decode_rename.push(now, decoded);
            if let Some(pc) = outcome.redirect {
                // What was fetched after the redirecting instruction is
                // wrong-path; fetch restarts at `pc` next cycle.
                self.fetch2_decode.clear();
                self.fetch1_fetch2.clear();
                engine.common_mut().drop_fetches();
                self.fetch_pc = pc;
                return;
            }
        }

        if self.fetch2_decode.is_empty()
            && let Some(mut landed) = self.fetch1_fetch2.ready(now).map(std::mem::take)
        {
            let mut decoded = Vec::new();
            fetch2::fetch2_stage(state, &mut landed, &mut decoded);
            self.fetch2_decode.push(now, decoded);
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
    }
}
