//! Frontend pipeline stages (shared across all backends).
//!
//! The frontend is generic over the execution engine and handles:
//! Fetch1 -> Fetch2 -> Decode -> Rename

pub mod decode;
pub mod fetch1;
pub mod fetch2;
pub mod rename;

use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::latches::{Fetch1Fetch2Entry, IdExEntry, IfIdEntry, RenameIssueEntry};
use std::marker::PhantomData;

/// The frontend pipeline, generic over the execution engine.
///
/// Same frontend code works with InOrderEngine and (future) O3Engine.
pub struct Frontend<E: ExecutionEngine> {
    /// Fetch1 -> Fetch2 latch.
    pub fetch1_fetch2: Vec<Fetch1Fetch2Entry>,
    /// Fetch2 -> Decode latch (reuses IfIdEntry for I-cache result).
    pub fetch2_decode: Vec<IfIdEntry>,
    /// Decode -> Rename latch (reuses IdExEntry for decoded signals).
    pub decode_rename: Vec<IdExEntry>,
    /// Fetch1 stall counter (I-TLB translation latency).
    pub fetch1_stall: u64,
    /// Fetch2 stall counter (I-cache / page-crossing latency).
    pub fetch2_stall: u64,
    /// Holding buffer for decoded instructions waiting on an I-cache miss.
    /// On a miss, fetch2 decodes into here and stalls; when the stall
    /// expires these are moved to fetch2_decode without re-accessing the
    /// I-cache (the line was already installed on the miss).
    fetch2_pending: Vec<IfIdEntry>,
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
            fetch2_pending: Vec::with_capacity(width),
            _marker: PhantomData,
        }
    }

    /// Executes one cycle of all frontend stages (reverse order).
    pub fn tick(
        &mut self,
        cpu: &mut crate::core::Cpu,
        engine: &mut E,
        rename_output: &mut Vec<RenameIssueEntry>,
    ) {
        // Rename: decode_rename -> engine (ROB alloc)
        rename::rename_stage(cpu, &mut self.decode_rename, engine, rename_output);

        // Decode: fetch2_decode -> decode_rename
        // Only run decode when rename has consumed the previous output;
        // otherwise decode would keep appending to decode_rename while
        // rename can't drain it (e.g. ROB full), causing unbounded growth
        // and O(n²) behaviour as rename re-scans the growing vec each cycle.
        if self.decode_rename.is_empty() {
            decode::decode_stage(cpu, &mut self.fetch2_decode, &mut self.decode_rename);
        }

        // Fetch2: fetch1_fetch2 -> fetch2_decode (gated by fetch2_stall or backpressure)
        if self.fetch2_stall > 0 {
            self.fetch2_stall -= 1;
            // When the stall expires, deliver pending instructions (from an
            // I-cache miss that already decoded but couldn't deliver).
            if self.fetch2_stall == 0 && !self.fetch2_pending.is_empty() {
                self.fetch2_decode.append(&mut self.fetch2_pending);
            }
        } else if self.fetch2_decode.is_empty() {
            fetch2::fetch2_stage(
                cpu,
                &mut self.fetch1_fetch2,
                &mut self.fetch2_decode,
                &mut self.fetch2_pending,
                &mut self.fetch2_stall,
            );
        }

        // Fetch1: PC gen -> fetch1_fetch2 (gated by fetch1_stall or backpressure)
        if self.fetch1_stall > 0 {
            self.fetch1_stall -= 1;
        } else if self.fetch1_fetch2.is_empty() {
            // Only run F1 when F2 has consumed the previous output;
            // otherwise F1 would clear the latch and overwrite entries
            // that F2 still needs to process.
            fetch1::fetch1_stage(cpu, &mut self.fetch1_fetch2, &mut self.fetch1_stall);
        }
    }

    /// Flushes all frontend latches and stall counters.
    pub fn flush(&mut self) {
        self.fetch1_fetch2.clear();
        self.fetch2_decode.clear();
        self.fetch2_pending.clear();
        self.decode_rename.clear();
        self.fetch1_stall = 0;
        self.fetch2_stall = 0;
    }
}
