//! RISC-V B-Extension Function Codes (funct3).
//!
//! The `funct3` field (bits 14-12) distinguishes between B-extension operations
//! that share the same `funct7` value.
//!
//! Encoding reference: RISC-V Bitmanip Extension v1.0.0, Chapter 2-5.

// ── Zba: Address generation ──────────────────────────────────────────────────

/// sh1add (funct3 for OP_REG with funct7 = SH_ADD).
pub const SH1ADD: u32 = 0b010;

/// sh2add (funct3 for OP_REG with funct7 = SH_ADD).
pub const SH2ADD: u32 = 0b100;

/// sh3add (funct3 for OP_REG with funct7 = SH_ADD).
pub const SH3ADD: u32 = 0b110;

/// add.uw / sh*add.uw share funct3 with sh*add; add.uw uses funct3 = 000.
pub const ADD_UW: u32 = 0b000;

/// sh1add.uw (OP_REG_32 with funct7 = SH_ADD).
pub const SH1ADD_UW: u32 = 0b010;

/// sh2add.uw (OP_REG_32 with funct7 = SH_ADD).
pub const SH2ADD_UW: u32 = 0b100;

/// sh3add.uw (OP_REG_32 with funct7 = SH_ADD).
pub const SH3ADD_UW: u32 = 0b110;

// ── Zbb: Basic bit manipulation ──────────────────────────────────────────────

/// andn (funct3 for OP_REG with funct7 = LOGICAL_NEG).
pub const ANDN: u32 = 0b111;

/// orn (funct3 for OP_REG with funct7 = LOGICAL_NEG).
pub const ORN: u32 = 0b110;

/// xnor (funct3 for OP_REG with funct7 = LOGICAL_NEG).
pub const XNOR: u32 = 0b100;

/// clz / ctz / cpop (funct3 for OP_IMM with funct7 = COUNT).
pub const COUNT_BITS: u32 = 0b001;

/// max (funct3 for OP_REG with funct7 = MIN_MAX).
pub const MAX: u32 = 0b110;

/// maxu (funct3 for OP_REG with funct7 = MIN_MAX).
pub const MAXU: u32 = 0b111;

/// min (funct3 for OP_REG with funct7 = MIN_MAX).
pub const MIN: u32 = 0b100;

/// minu (funct3 for OP_REG with funct7 = MIN_MAX).
pub const MINU: u32 = 0b101;

/// rol (funct3 for OP_REG with funct7 = ROTATE).
pub const ROL: u32 = 0b001;

/// ror / rori (funct3 for OP_REG/OP_IMM with funct7 = ROTATE_RIGHT).
pub const ROR: u32 = 0b101;

/// orc.b / rev8 / sext.b / sext.h share funct3 = 0b101 on OP_IMM.
pub const ORC_REV_SEXT: u32 = 0b101;

/// zext.h (funct3 for OP_REG_32 with funct7 = ZEXT_H).
pub const ZEXT_H: u32 = 0b100;

// ── Zbc: Carry-less multiplication ───────────────────────────────────────────

/// clmul (funct3 for OP_REG with funct7 = CLMUL).
pub const CLMUL: u32 = 0b001;

/// clmulh (funct3 for OP_REG with funct7 = CLMUL).
pub const CLMULH: u32 = 0b011;

/// clmulr (funct3 for OP_REG with funct7 = CLMUL).
pub const CLMULR: u32 = 0b010;

// ── Zbs: Single-bit operations ───────────────────────────────────────────────

/// bclr / bclri (funct3 for OP_REG/OP_IMM with funct7 = BCLR).
pub const BCLR: u32 = 0b001;

/// bext / bexti (funct3 for OP_REG/OP_IMM with funct7 = BEXT).
pub const BEXT: u32 = 0b101;

/// binv / binvi (funct3 for OP_REG/OP_IMM with funct7 = BINV).
pub const BINV: u32 = 0b001;

/// bset / bseti (funct3 for OP_REG/OP_IMM with funct7 = BSET).
pub const BSET: u32 = 0b001;

// ── Zbkb: Bitwise operations for cryptography ───────────────────────────────

/// pack (funct3 for OP_REG with funct7 = PACK).
pub const PACK: u32 = 0b100;

/// packh (funct3 for OP_REG with funct7 = PACK).
pub const PACKH: u32 = 0b111;

/// packw (funct3 for OP_REG_32 with funct7 = PACK).
pub const PACKW: u32 = 0b100;

// ── Zbkx: Crossbar permutations for cryptography ────────────────────────────

/// xperm4 (funct3 for OP_REG with funct7 = XPERM).
pub const XPERM4: u32 = 0b010;

/// xperm8 (funct3 for OP_REG with funct7 = XPERM).
pub const XPERM8: u32 = 0b100;

// ── Full I-type immediate encodings for unary instructions ───────────────────
//
// Several B-extension instructions are encoded as I-type with a fixed
// immediate (funct7 || rs2 = imm[11:0]). The spec defines these as
// complete 12-bit immediates rather than separate funct7/rs2 fields.
// We extract the unsigned imm[11:0] from the raw instruction at decode time.

/// Mask to extract the unsigned I-type immediate (bits 31:20) from raw instruction.
pub const I_IMM_MASK: u32 = 0xFFF0_0000;
/// Shift to bring I-type immediate to bits [11:0].
pub const I_IMM_SHIFT: u32 = 20;

/// clz: imm[11:0] = 0x600 (funct7=0b0110000, rs2=0b00000).
pub const CLZ_IMM: u32 = 0x600;

/// ctz: imm[11:0] = 0x601 (funct7=0b0110000, rs2=0b00001).
pub const CTZ_IMM: u32 = 0x601;

/// cpop: imm[11:0] = 0x602 (funct7=0b0110000, rs2=0b00010).
pub const CPOP_IMM: u32 = 0x602;

/// sext.b: imm[11:0] = 0x604 (funct7=0b0110000, rs2=0b00100).
pub const SEXT_B_IMM: u32 = 0x604;

/// sext.h: imm[11:0] = 0x605 (funct7=0b0110000, rs2=0b00101).
pub const SEXT_H_IMM: u32 = 0x605;

/// orc.b: imm[11:0] = 0x287 (funct7=0b0010100, rs2=0b00111).
pub const ORC_B_IMM: u32 = 0x287;

/// rev8 (RV64): imm[11:0] = 0x6B8 (funct7=0b0110101, rs2=0b11000).
pub const REV8_IMM: u32 = 0x6B8;

/// brev8 (Zbkb): imm[11:0] = 0x687 (funct7=0b0110100, rs2=0b00111).
/// Bit-reverse within each byte. Encoded as I-type with funct3 = SRL_SRA.
pub const BREV8_IMM: u32 = 0x687;
