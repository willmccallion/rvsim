//! Execution engine traits and pipeline type erasure.
//!
//! This module defines the trait hierarchy for pluggable backends:
//! 1. **`IssueUnit`** — stage-level trait for instruction issue (FIFO vs O3).
//! 2. **`ExecuteUnit`** — stage-level trait for instruction execution.
//! 3. **`ExecutionEngine`** — high-level trait covering the entire backend.
//! 4. **`PipelineDispatch`** — enum dispatch for type-erased pipeline storage.

use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::scoreboard::Scoreboard;
use crate::core::pipeline::store_buffer::StoreBuffer;
use serde::Deserialize;

/// Backend type selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum BackendType {
    /// In-order pipeline (default).
    #[default]
    InOrder,
    /// Out-of-order pipeline (future).
    OutOfOrder,
}

/// The execution engine trait — implemented by InOrderEngine (and O3Engine in the future).
///
/// Covers the backend pipeline: Issue -> Execute -> Memory1 -> Memory2 -> Writeback -> Commit.
pub trait ExecutionEngine {
    /// Run one cycle of all backend stages (reverse order internally).
    fn tick(&mut self, cpu: &mut crate::core::Cpu, rename_output: &mut Vec<RenameIssueEntry>);

    /// How many instructions can the engine accept from rename this cycle?
    fn can_accept(&self) -> usize;

    /// Flush all speculative state. Committed stores in the store buffer remain.
    fn flush(&mut self, cpu: &mut crate::core::Cpu);

    /// Read a CSR, checking in-flight CsrUpdate entries in the ROB.
    fn read_csr_speculative(&self, cpu: &crate::core::Cpu, addr: u32) -> u64;

    /// Access the scoreboard (for rename to mark producers, issue to check readiness).
    fn scoreboard(&self) -> &Scoreboard;
    fn scoreboard_mut(&mut self) -> &mut Scoreboard;

    /// Access the ROB (for rename to allocate entries, forwarding, etc.).
    fn rob(&self) -> &Rob;
    fn rob_mut(&mut self) -> &mut Rob;

    /// Access the store buffer (for rename to allocate, memory2 for forwarding).
    fn store_buffer(&self) -> &StoreBuffer;
    fn store_buffer_mut(&mut self) -> &mut StoreBuffer;
}

/// The full pipeline combines a frontend and an engine.
///
/// The frontend is generic over the engine type, so the full pipeline
/// maintains both together.
pub struct Pipeline<E: ExecutionEngine> {
    pub frontend: crate::core::pipeline::frontend::Frontend<E>,
    pub engine: E,
    /// Buffer for rename stage output, consumed by the engine each cycle.
    pub rename_output: Vec<RenameIssueEntry>,
}

impl<E: ExecutionEngine> Pipeline<E> {
    /// Run one cycle of the entire pipeline.
    pub fn tick(&mut self, cpu: &mut crate::core::Cpu) {
        let pc_before = cpu.pc;

        // Backend always runs (commit/writeback/memory must drain even during stalls)
        self.engine.tick(cpu, &mut self.rename_output);

        // If the backend redirected the PC (branch misprediction, trap, etc.),
        // flush the frontend and any pending rename output so stale instructions
        // from the old PC stream don't leak into the backend next cycle.
        // (The backend's own flush path clears rename_output for execute-detected
        // redirects, but the trap path in commit returns early before reaching
        // that code.)
        if cpu.pc != pc_before {
            self.frontend.flush();
            self.rename_output.clear();
        }

        // Frontend runs every cycle (per-stage stalls are handled internally)
        if cpu.exit_code.is_none() && !cpu.wfi_waiting {
            self.frontend
                .tick(cpu, &mut self.engine, &mut self.rename_output);
        }
    }

    /// Flush the entire pipeline.
    pub fn flush(&mut self, cpu: &mut crate::core::Cpu) {
        self.frontend.flush();
        self.rename_output.clear();
        self.engine.flush(cpu);
    }
}

/// Type-erased pipeline for storage in the non-generic Cpu struct.
pub enum PipelineDispatch {
    /// In-order pipeline.
    InOrder(Box<Pipeline<crate::core::pipeline::backend::inorder::InOrderEngine>>),
    /// Out-of-order pipeline.
    OutOfOrder,
}

impl PipelineDispatch {
    /// Run one cycle.
    pub fn tick(&mut self, cpu: &mut crate::core::Cpu) {
        match self {
            Self::InOrder(p) => p.tick(cpu),
            Self::OutOfOrder => unimplemented!("out-of-order pipeline"),
        }
    }

    /// Flush.
    pub fn flush(&mut self, cpu: &mut crate::core::Cpu) {
        match self {
            Self::InOrder(p) => p.flush(cpu),
            Self::OutOfOrder => unimplemented!("out-of-order pipeline"),
        }
    }
}
