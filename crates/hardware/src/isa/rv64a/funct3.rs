//! RISC-V Atomic Extension (A) Function Codes (funct3).
//!
//! The `funct3` field in AMO instructions encodes the operation width and
//! ordering constraints.

/// Operation Width: 32-bit (Word).
pub const WIDTH_32: u32 = 0b010;

/// Operation Width: 64-bit (Double).
pub const WIDTH_64: u32 = 0b011;

/// Ordering: Acquire.
pub const AQ: u32 = 1 << 1;

/// Ordering: Release.
pub const RL: u32 = 1 << 0;
