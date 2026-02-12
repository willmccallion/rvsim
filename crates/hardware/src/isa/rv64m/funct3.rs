//! RISC-V M-Extension Function Codes (funct3).
//!
//! Identifies the specific multiply or divide operation when `opcode == OP_REG`
//! and `funct7 == 1`.

/// Multiply (signed * signed) -> lower 64 bits.
pub const MUL: u32 = 0b000;

/// Multiply High (signed * signed) -> upper 64 bits.
pub const MULH: u32 = 0b001;

/// Multiply High Signed/Unsigned (signed * unsigned) -> upper 64 bits.
pub const MULHSU: u32 = 0b010;

/// Multiply High Unsigned (unsigned * unsigned) -> upper 64 bits.
pub const MULHU: u32 = 0b011;

/// Divide (signed).
pub const DIV: u32 = 0b100;

/// Divide Unsigned.
pub const DIVU: u32 = 0b101;

/// Remainder (signed).
pub const REM: u32 = 0b110;

/// Remainder Unsigned.
pub const REMU: u32 = 0b111;
