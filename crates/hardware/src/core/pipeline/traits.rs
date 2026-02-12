//! Pipeline Stage and Latch Interfaces.
//!
//! This module defines the common traits for pipeline components. It provides:
//! 1. **Pipeline Stage Interface:** Standardizes the `tick` operation for all stages.
//! 2. **Pipeline Latch Interface:** Provides methods for flushing and status checking.

/// Represents a stage in the instruction pipeline.
///
/// A stage is responsible for processing a specific part of the instruction
/// lifecycle (Fetch, Decode, Execute, Memory, or Writeback).
pub trait PipelineStage {
    /// Executes one cycle of the pipeline stage.
    ///
    /// # Arguments
    ///
    /// * `cpu` - Mutable reference to the CPU state.
    fn tick(cpu: &mut crate::core::Cpu);
}

/// Represents a pipeline latch (inter-stage buffer).
///
/// Latches hold the state of instructions as they move between stages. They support
/// flushing and status checks.
pub trait PipelineLatch {
    /// Clears all entries in the latch.
    ///
    /// Typically called when a branch misprediction or trap occurs.
    fn flush(&mut self);

    /// Checks if the latch is empty.
    ///
    /// # Returns
    ///
    /// `true` if there are no valid instructions in the latch, `false` otherwise.
    fn is_empty(&self) -> bool;

    /// Checks if the latch contains any instruction that has triggered a trap.
    ///
    /// # Returns
    ///
    /// `true` if any entry in the latch has a pending trap.
    fn has_trap(&self) -> bool;
}
