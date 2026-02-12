//! RISC-V Atomic Extension (A).
//!
//! Defines constants and logic for Atomic Memory Operations (AMO).
//! AMOs perform a read-modify-write operation in a single instruction.

/// Function code 3 definitions for atomic operation types.
pub mod funct3;

/// Function code 5 definitions for atomic operation variants.
pub mod funct5;

/// Atomic extension opcodes (AMO, LR, SC).
pub mod opcodes;
