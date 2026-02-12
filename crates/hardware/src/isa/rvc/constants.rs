//! RISC-V Compressed (C) Extension Constants.
//!
//! Defines the quadrants and opcodes for 16-bit compressed instructions.
//! Compressed instructions are divided into three quadrants (0, 1, 2) based
//! on the lowest 2 bits of the instruction.

/// Quadrant 0 (bits 1:0 = 00).
pub const QUADRANT_0: u16 = 0b00;
/// Quadrant 1 (bits 1:0 = 01).
pub const QUADRANT_1: u16 = 0b01;
/// Quadrant 2 (bits 1:0 = 10).
pub const QUADRANT_2: u16 = 0b10;

/// Instructions in Quadrant 0.
pub mod q0 {
    /// Compressed Add Immediate, scaled by 4, to Stack Pointer (C.ADDI4SPN).
    pub const C_ADDI4SPN: u16 = 0b000;
    /// Compressed Floating-point Load Double (C.FLD).
    pub const C_FLD: u16 = 0b001;
    /// Compressed Load Word (C.LW).
    pub const C_LW: u16 = 0b010;
    /// Compressed Load Double (C.LD).
    pub const C_LD: u16 = 0b011;
    /// Compressed Floating-point Store Double (C.FSD).
    pub const C_FSD: u16 = 0b101;
    /// Compressed Store Word (C.SW).
    pub const C_SW: u16 = 0b110;
    /// Compressed Store Double (C.SD).
    pub const C_SD: u16 = 0b111;
}

/// Instructions in Quadrant 1.
pub mod q1 {
    /// Compressed Add Immediate (C.ADDI).
    pub const C_ADDI: u16 = 0b000;
    /// Compressed Add Immediate Word (C.ADDIW).
    pub const C_ADDIW: u16 = 0b001;
    /// Compressed Load Immediate (C.LI).
    pub const C_LI: u16 = 0b010;
    /// Compressed Load Upper Immediate / Add Immediate 16 to SP (C.LUI / C.ADDI16SP).
    pub const C_LUI_ADDI16SP: u16 = 0b011;
    /// Miscellaneous ALU operations (C.SRLI, C.SRAI, C.ANDI, C.SUB, etc.).
    pub const C_MISC_ALU: u16 = 0b100;
    /// Compressed Jump (C.J).
    pub const C_J: u16 = 0b101;
    /// Compressed Branch Equal Zero (C.BEQZ).
    pub const C_BEQZ: u16 = 0b110;
    /// Compressed Branch Not Equal Zero (C.BNEZ).
    pub const C_BNEZ: u16 = 0b111;
}

/// Instructions in Quadrant 2.
pub mod q2 {
    /// Compressed Shift Left Logical Immediate (C.SLLI).
    pub const C_SLLI: u16 = 0b000;
    /// Compressed Floating-point Load Double from SP (C.FLDSP).
    pub const C_FLDSP: u16 = 0b001;
    /// Compressed Load Word from SP (C.LWSP).
    pub const C_LWSP: u16 = 0b010;
    /// Compressed Load Double from SP (C.LDSP).
    pub const C_LDSP: u16 = 0b011;
    /// Miscellaneous ALU / Jump (C.JR, C.MV, C.EBREAK, C.JALR, C.ADD).
    pub const C_MISC_ALU: u16 = 0b100;
    /// Compressed Floating-point Store Double to SP (C.FSDSP).
    pub const C_FSDSP: u16 = 0b101;
    /// Compressed Store Word to SP (C.SWSP).
    pub const C_SWSP: u16 = 0b110;
    /// Compressed Store Double to SP (C.SDSP).
    pub const C_SDSP: u16 = 0b111;
}
