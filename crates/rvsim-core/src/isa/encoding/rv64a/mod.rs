//! RISC-V Atomic Extension (A).
//!
//! Defines constants and logic for Atomic Memory Operations (AMO).
//! AMOs perform a read-modify-write operation in a single instruction.

pub mod funct3;

pub mod funct5;

pub mod opcodes;

/// The `aq` (acquire) ordering bit of an AMO, LR or SC instruction.
pub const AQ: u32 = 1 << 26;

/// The `rl` (release) ordering bit of an AMO, LR or SC instruction.
pub const RL: u32 = 1 << 25;
