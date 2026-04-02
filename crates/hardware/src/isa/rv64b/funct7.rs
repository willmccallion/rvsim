//! RISC-V B-Extension Function Codes (funct7).
//!
//! The `funct7` field (bits 31-25) distinguishes B-extension instructions
//! from base integer and M-extension instructions that share the same opcodes.
//!
//! Encoding reference: RISC-V Bitmanip Extension v1.0.0, Chapter 2-5.

// ── Zba: Address generation ──────────────────────────────────────────────────

/// sh1add, sh2add, sh3add (OP_REG funct7).
pub const SH_ADD: u32 = 0b0010000;

/// add.uw (OP_REG_32 funct7).
pub const ADD_UW: u32 = 0b0000100;

/// slli.uw (OP_IMM_32, funct3=001, top 6 bits of funct7).
/// Full funct7 = 0b0000100, but only top 6 bits are checked for shifts.
pub const SLLI_UW: u32 = 0b0000100;

// ── Zbb: Basic bit manipulation ──────────────────────────────────────────────

/// andn, orn, xnor (OP_REG funct7).
pub const LOGICAL_NEG: u32 = 0b0100000;

/// clz, ctz, cpop (OP_IMM funct7 = 0b0110000, with rs2 encoding the specific op).
pub const COUNT: u32 = 0b0110000;

/// sext.b, sext.h (OP_IMM funct7 = 0b0110000, with specific rs2 values).
/// Same funct7 as COUNT — differentiated by funct3.
pub const SIGN_EXTEND: u32 = 0b0110000;

/// max, maxu, min, minu (OP_REG funct7).
pub const MIN_MAX: u32 = 0b0000101;

/// rol (OP_REG funct7).
pub const ROTATE: u32 = 0b0110000;

/// ror / rori (OP_REG / OP_IMM funct7).
/// For ror: funct7 = 0b0110000.
/// For rori: top 6 bits = 0b011000 (bit 25 holds shamt[5]).
pub const ROTATE_RIGHT: u32 = 0b0110000;

/// orc.b (OP_IMM funct7 = 0b0010100, rs2 = 0b00111).
pub const ORC_B: u32 = 0b0010100;

/// rev8 (OP_IMM funct7: on RV64, imm[11:0] = 0b011010111000).
/// funct7 = 0b0110101, rs2 = 0b11000.
pub const REV8: u32 = 0b0110101;

/// zext.h (OP_REG_32 funct7 = 0b0000100, funct3 = 0b100).
pub const ZEXT_H: u32 = 0b0000100;

// ── Zbc: Carry-less multiplication ───────────────────────────────────────────

/// clmul, clmulh, clmulr (OP_REG funct7).
pub const CLMUL: u32 = 0b0000101;

// ── Zbs: Single-bit operations ───────────────────────────────────────────────

/// bclr / bclri (OP_REG / OP_IMM funct7).
pub const BCLR: u32 = 0b0100100;

/// bext / bexti (OP_REG / OP_IMM funct7).
pub const BEXT: u32 = 0b0100100;

/// binv / binvi (OP_REG / OP_IMM funct7).
pub const BINV: u32 = 0b0110100;

/// bset / bseti (OP_REG / OP_IMM funct7).
pub const BSET: u32 = 0b0010100;
