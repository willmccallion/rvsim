//! Core processor implementation.
//!
//! This module contains the main CPU implementation including the instruction
//! pipeline, execution units, architecture-specific components, and the
//! orchestrator that coordinates all components.

/// Architecture-specific components (CSRs, register files, privilege modes, traps).
pub mod arch;

/// CPU core implementation and execution orchestration.
pub mod cpu;

/// Instruction pipeline implementation (stages, latches, hazards, signals).
pub mod pipeline;

/// Execution units (ALU, FPU, LSU, MMU, branch predictor, cache, prefetcher).
pub mod units;

pub use self::cpu::Cpu;
