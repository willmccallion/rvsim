//! RISC-V Base Integer (I) Function Codes (funct7).
//!
//! The `funct7` field (bits 31-25) is used in R-type instructions to
//! distinguish between operations that share the same `funct3` (e.g., ADD vs SUB).

/// Default operation (ADD, SRL, etc.).
pub const DEFAULT: u32 = 0b0000000;

/// Alternate operation (SUB, SRA).
/// Used to distinguish SUB from ADD, and SRA from SRL.
pub const SUB: u32 = 0b0100000;
/// Alias for SUB (used for Shift Right Arithmetic).
pub const SRA: u32 = 0b0100000;
