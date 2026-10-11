//! Pipeline traces: where every instruction is each cycle, for a viewer.
//!
//! The pipeline reports each instruction's progress through a
//! [`PipelineTrace`]: fetched, decoded, the stage it enters, and how it
//! leaves. A trace format is one implementation; [`kanata`] writes the
//! Kanata log Konata displays, and another viewer's format is another
//! implementation of the same trait.

pub mod kanata;

use crate::common::{HartId, InstSeq, SimError};

/// A stage an instruction occupies, as a viewer shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Fetching its line, through fetch1 and fetch2.
    Fetch,
    Decode,
    /// Renamed and placed in the reorder buffer.
    Rename,
    /// Dispatched into the issue queue (the out-of-order backend).
    Dispatch,
    /// Selected to issue (the out-of-order backend).
    Issue,
    Execute,
    /// In the memory stages, translating and accessing.
    Memory,
    /// Its result written back, waiting to commit.
    Writeback,
    Commit,
}

/// How an instruction left the pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Departure {
    Retired,
    Squashed,
}

/// The formats a pipeline trace can be written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineTraceFormat {
    /// The Kanata log Konata displays.
    Kanata,
}

/// What a pipeline trace is told. Every call carries the cycle it happens
/// in; an instruction is `seq` on `hart` from fetch to retirement.
pub trait PipelineTrace: Send + Sync {
    /// `seq` entered the pipeline, fetching `pc`.
    fn fetched(&mut self, cycle: u64, hart: HartId, seq: InstSeq, pc: u64);

    /// `seq`'s encoding is known.
    fn decoded(&mut self, cycle: u64, hart: HartId, seq: InstSeq, inst: u32);

    /// `seq` moved into `stage`.
    fn entered(&mut self, cycle: u64, hart: HartId, seq: InstSeq, stage: Stage);

    /// `seq` left the pipeline.
    fn left(&mut self, cycle: u64, hart: HartId, seq: InstSeq, how: Departure);

    /// Writes out what is buffered.
    ///
    /// # Errors
    ///
    /// The trace file could not be written.
    fn flush(&mut self) -> Result<(), SimError>;
}

impl std::fmt::Debug for dyn PipelineTrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PipelineTrace")
    }
}
