//! The Kanata log Konata displays.
//!
//! A log starts with `Kanata\t0004`, then one command per line: `C=\t<cycle>`
//! sets the cycle and `C\t<n>` advances it; `I\t<id>\t<seq>\t<thread>`
//! starts an instruction, `L\t<id>\t<type>\t<text>` labels it (type 0 is
//! the text in its row, 1 the detail shown on hover), `S\t<id>\t0\t<stage>`
//! and `E\t<id>\t0\t<stage>` open and close a stage in lane 0, and
//! `R\t<id>\t<retire id>\t<type>` ends it, type 0 retired and 1 flushed.
//! Instructions are numbered in the order they are fetched; a hart is a
//! thread.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};

use super::{Departure, PipelineTrace, Stage};
use crate::common::{HartId, InstSeq, SimError};
use crate::isa::disasm::disassemble;

const HEADER: &str = "Kanata\t0004\n";

/// An instruction in the pipeline: its Kanata id and the stage it is in.
struct Live {
    id: u64,
    stage: Option<Stage>,
}

/// Writes a Kanata log to `out`.
pub struct KanataWriter<W: Write> {
    out: W,
    path: String,
    /// The cycle the last command was written in; `None` before the first.
    cycle: Option<u64>,
    live: HashMap<(HartId, InstSeq), Live>,
    next_id: u64,
    retired: u64,
}

impl KanataWriter<BufWriter<File>> {
    /// Creates the log at `path`.
    ///
    /// # Errors
    ///
    /// The file could not be created or its header written.
    pub fn create(path: &str) -> Result<Self, SimError> {
        let file = File::create(path).map_err(|source| file_error(path, source))?;
        Self::new(BufWriter::with_capacity(1 << 20, file), path)
    }
}

impl<W: Write> KanataWriter<W> {
    /// A log written to `out`, named `path` in errors.
    ///
    /// # Errors
    ///
    /// The header could not be written.
    pub fn new(mut out: W, path: &str) -> Result<Self, SimError> {
        out.write_all(HEADER.as_bytes()).map_err(|source| file_error(path, source))?;
        Ok(Self {
            out,
            path: path.to_owned(),
            cycle: None,
            live: HashMap::new(),
            next_id: 0,
            retired: 0,
        })
    }

    /// The writer's output, for a test that gave it a buffer.
    #[cfg(test)]
    pub fn into_inner(self) -> W {
        self.out
    }

    /// Brings the log's cycle to `cycle`.
    fn at(&mut self, cycle: u64) -> io::Result<()> {
        match self.cycle {
            Some(current) if current == cycle => Ok(()),
            Some(current) if cycle > current => {
                self.cycle = Some(cycle);
                writeln!(self.out, "C\t{}", cycle - current)
            }
            _ => {
                self.cycle = Some(cycle);
                writeln!(self.out, "C=\t{cycle}")
            }
        }
    }

    fn close_stage(out: &mut W, live: &mut Live) -> io::Result<()> {
        if let Some(stage) = live.stage.take() {
            writeln!(out, "E\t{}\t0\t{}", live.id, label(stage))?;
        }
        Ok(())
    }

    fn fetched_at(&mut self, cycle: u64, hart: HartId, seq: InstSeq, pc: u64) -> io::Result<()> {
        self.at(cycle)?;
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.out, "I\t{id}\t{}\t{}", seq.val(), hart.val())?;
        writeln!(self.out, "L\t{id}\t0\t{pc:#x}: ")?;
        writeln!(self.out, "L\t{id}\t1\thart {} seq {}", hart.val(), seq.val())?;
        writeln!(self.out, "S\t{id}\t0\t{}", label(Stage::Fetch))?;
        if self.live.insert((hart, seq), Live { id, stage: Some(Stage::Fetch) }).is_some() {
            tracing::debug!(target: "rvsim::trace", hart = hart.val(), seq = seq.val(), "instruction fetched twice while in flight");
        }
        Ok(())
    }

    fn decoded_at(&mut self, cycle: u64, hart: HartId, seq: InstSeq, inst: u32) -> io::Result<()> {
        let Some(live) = self.live.get(&(hart, seq)) else { return Ok(()) };
        let id = live.id;
        self.at(cycle)?;
        writeln!(self.out, "L\t{id}\t0\t{}", disassemble(inst))?;
        writeln!(self.out, "L\t{id}\t1\t inst {inst:#010x}")
    }

    fn entered_at(
        &mut self,
        cycle: u64,
        hart: HartId,
        seq: InstSeq,
        stage: Stage,
    ) -> io::Result<()> {
        let Some(live) = self.live.get_mut(&(hart, seq)) else { return Ok(()) };
        if live.stage == Some(stage) {
            return Ok(());
        }
        self.at(cycle)?;
        let live = self.live.get_mut(&(hart, seq)).ok_or(io::ErrorKind::NotFound)?;
        Self::close_stage(&mut self.out, live)?;
        live.stage = Some(stage);
        writeln!(self.out, "S\t{}\t0\t{}", live.id, label(stage))
    }

    fn left_at(
        &mut self,
        cycle: u64,
        hart: HartId,
        seq: InstSeq,
        how: Departure,
    ) -> io::Result<()> {
        let Some(mut live) = self.live.remove(&(hart, seq)) else { return Ok(()) };
        self.at(cycle)?;
        Self::close_stage(&mut self.out, &mut live)?;
        let kind = match how {
            Departure::Retired => {
                self.retired += 1;
                0
            }
            Departure::Squashed => 1,
        };
        writeln!(self.out, "R\t{}\t{}\t{kind}", live.id, self.retired)
    }

    fn report(&self, result: io::Result<()>) {
        if let Err(error) = result {
            tracing::warn!(target: "rvsim::trace", path = %self.path, %error, "pipeline trace not written");
        }
    }
}

/// The stage's name in the log, short enough for a cell.
const fn label(stage: Stage) -> &'static str {
    match stage {
        Stage::Fetch => "F",
        Stage::Decode => "Dc",
        Stage::Rename => "Rn",
        Stage::Dispatch => "Ds",
        Stage::Issue => "Is",
        Stage::Execute => "Ex",
        Stage::Memory => "Mem",
        Stage::Writeback => "Wb",
        Stage::Commit => "Cm",
    }
}

fn file_error(path: &str, source: io::Error) -> SimError {
    SimError::FileRead { path: path.to_owned(), source }
}

impl<W: Write + Send + Sync> PipelineTrace for KanataWriter<W> {
    fn fetched(&mut self, cycle: u64, hart: HartId, seq: InstSeq, pc: u64) {
        let result = self.fetched_at(cycle, hart, seq, pc);
        self.report(result);
    }

    fn decoded(&mut self, cycle: u64, hart: HartId, seq: InstSeq, inst: u32) {
        let result = self.decoded_at(cycle, hart, seq, inst);
        self.report(result);
    }

    fn entered(&mut self, cycle: u64, hart: HartId, seq: InstSeq, stage: Stage) {
        let result = self.entered_at(cycle, hart, seq, stage);
        self.report(result);
    }

    fn left(&mut self, cycle: u64, hart: HartId, seq: InstSeq, how: Departure) {
        let result = self.left_at(cycle, hart, seq, how);
        self.report(result);
    }

    fn flush(&mut self) -> Result<(), SimError> {
        self.out.flush().map_err(|source| file_error(&self.path, source))
    }
}
