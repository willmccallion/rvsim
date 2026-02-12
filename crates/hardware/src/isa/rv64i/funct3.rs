//! RISC-V Base Integer (I) Function Codes (funct3).
//!
//! The `funct3` field (bits 14-12) distinguishes between instructions sharing
//! the same major opcode (e.g., LB vs LH, BEQ vs BNE, ADD vs SLT).

/// Load Byte (signed).
pub const LB: u32 = 0b000;
/// Load Halfword (signed).
pub const LH: u32 = 0b001;
/// Load Word (signed).
pub const LW: u32 = 0b010;
/// Load Doubleword.
pub const LD: u32 = 0b011;
/// Load Byte Unsigned.
pub const LBU: u32 = 0b100;
/// Load Halfword Unsigned.
pub const LHU: u32 = 0b101;
/// Load Word Unsigned.
pub const LWU: u32 = 0b110;

/// Store Byte.
pub const SB: u32 = 0b000;
/// Store Halfword.
pub const SH: u32 = 0b001;
/// Store Word.
pub const SW: u32 = 0b010;
/// Store Doubleword.
pub const SD: u32 = 0b011;

/// Branch Equal.
pub const BEQ: u32 = 0b000;
/// Branch Not Equal.
pub const BNE: u32 = 0b001;
/// Branch Less Than (signed).
pub const BLT: u32 = 0b100;
/// Branch Greater or Equal (signed).
pub const BGE: u32 = 0b101;
/// Branch Less Than Unsigned.
pub const BLTU: u32 = 0b110;
/// Branch Greater or Equal Unsigned.
pub const BGEU: u32 = 0b111;

/// Add / Subtract.
pub const ADD_SUB: u32 = 0b000;
/// Shift Left Logical.
pub const SLL: u32 = 0b001;
/// Set Less Than (signed).
pub const SLT: u32 = 0b010;
/// Set Less Than Unsigned.
pub const SLTU: u32 = 0b011;
/// Bitwise XOR.
pub const XOR: u32 = 0b100;
/// Shift Right Logical / Arithmetic.
pub const SRL_SRA: u32 = 0b101;
/// Bitwise OR.
pub const OR: u32 = 0b110;
/// Bitwise AND.
pub const AND: u32 = 0b111;

/// Fence.
pub const FENCE: u32 = 0b000;
/// Instruction Fence.
pub const FENCE_I: u32 = 0b001;
