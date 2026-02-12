//! Load/Store Unit (LSU).
//!
//! This module provides the Load/Store Unit, responsible for memory access
//! operations. It includes:
//! - [`atomic`]: Read-modify-write ALU for the RISC-V A extension.
//! - [`ordering`]: Memory ordering / fence support (stub).
//! - [`unaligned`]: Unaligned access handling (stub).

/// Atomic memory operation ALU (RISC-V A extension).
pub mod atomic;

/// Memory ordering and fence operations (stub, Phase 3).
pub mod ordering;

/// Unaligned memory access handling (stub, Phase 3).
pub mod unaligned;

use crate::core::pipeline::signals::{AtomicOp, MemWidth};

/// Load/Store Unit (LSU) for memory operations.
///
/// Provides a unified interface for atomic memory operations.
/// Load/store pipeline integration is handled by the memory stage.
pub struct Lsu;

impl Lsu {
    /// Performs an atomic ALU operation for atomic memory instructions.
    ///
    /// Delegates to [`atomic::atomic_alu`]. See that function for full
    /// documentation.
    ///
    /// # Arguments
    ///
    /// * `op`      - The atomic operation type
    /// * `mem_val` - The current value read from memory
    /// * `reg_val` - The value from the source register
    /// * `width`   - The width of the operation (Word or Double)
    ///
    /// # Returns
    ///
    /// The computed result that will be written back to memory.
    pub fn atomic_alu(op: AtomicOp, mem_val: u64, reg_val: u64, width: MemWidth) -> u64 {
        atomic::atomic_alu(op, mem_val, reg_val, width)
    }
}
