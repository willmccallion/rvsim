//! The arithmetic the functional units perform, with no timing.

/// Integer ALU operations.
pub mod alu;

/// Atomic memory operation arithmetic (the A extension's read-modify-write).
pub mod amo;

/// Scalar floating-point operations.
pub mod fpu;

/// Misaligned access checks, traps, and byte-wise splitting.
pub mod misaligned;

/// Vector (RVV 1.0) execution.
pub mod vector;
