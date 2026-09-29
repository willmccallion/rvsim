//! RISC-V Atomic Extension (A) Function Codes (funct5).
//!
//! The `funct5` field (bits 31-27) specifies the atomic operation to perform.

/// Atomic Load-Reserved.
pub const LR: u32 = 0b00010;

/// Atomic Store-Conditional.
pub const SC: u32 = 0b00011;

/// Atomic Swap.
pub const AMOSWAP: u32 = 0b00001;

/// Atomic Add.
pub const AMOADD: u32 = 0b00000;

/// Atomic XOR.
pub const AMOXOR: u32 = 0b00100;

/// Atomic AND.
pub const AMOAND: u32 = 0b01100;

/// Atomic OR.
pub const AMOOR: u32 = 0b01000;

/// Atomic Minimum (Signed).
pub const AMOMIN: u32 = 0b10000;

/// Atomic Maximum (Signed).
pub const AMOMAX: u32 = 0b10100;

/// Atomic Minimum (Unsigned).
pub const AMOMINU: u32 = 0b11000;

/// Atomic Maximum (Unsigned).
pub const AMOMAXU: u32 = 0b11100;
