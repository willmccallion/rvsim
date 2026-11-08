//! Encoders for the 32-bit instruction formats, the inverse of
//! [`decode`](super::instruction::decode): what compressed-instruction
//! expansion and the tests build instruction words with.

use super::reg::RegIdx;

/// An R-type instruction: `funct7 | rs2 | rs1 | funct3 | rd | opcode`.
#[derive(Clone, Copy, Debug)]
pub struct RType {
    /// The major opcode.
    pub opcode: u32,
    /// The minor opcode in bits 14:12.
    pub funct3: u32,
    /// The minor opcode in bits 31:25.
    pub funct7: u32,
    /// The destination.
    pub rd: RegIdx,
    /// The first source.
    pub rs1: RegIdx,
    /// The second source.
    pub rs2: RegIdx,
}

impl RType {
    /// The instruction word.
    #[must_use]
    pub const fn encode(self) -> u32 {
        (self.funct7 & 0x7F) << 25
            | reg(self.rs2) << 20
            | reg(self.rs1) << 15
            | (self.funct3 & 0x7) << 12
            | reg(self.rd) << 7
            | (self.opcode & 0x7F)
    }
}

/// An R4-type instruction (the fused multiply-adds):
/// `rs3 | fmt | rs2 | rs1 | funct3 | rd | opcode`.
#[derive(Clone, Copy, Debug)]
pub struct R4Type {
    /// The major opcode.
    pub opcode: u32,
    /// The rounding mode or minor opcode in bits 14:12.
    pub funct3: u32,
    /// The format in bits 26:25.
    pub fmt: u32,
    /// The destination.
    pub rd: RegIdx,
    /// The first source.
    pub rs1: RegIdx,
    /// The second source.
    pub rs2: RegIdx,
    /// The third source.
    pub rs3: RegIdx,
}

impl R4Type {
    /// The instruction word.
    #[must_use]
    pub const fn encode(self) -> u32 {
        reg(self.rs3) << 27
            | (self.fmt & 0x3) << 25
            | reg(self.rs2) << 20
            | reg(self.rs1) << 15
            | (self.funct3 & 0x7) << 12
            | reg(self.rd) << 7
            | (self.opcode & 0x7F)
    }
}

/// An I-type instruction: `imm[11:0] | rs1 | funct3 | rd | opcode`.
#[derive(Clone, Copy, Debug)]
pub struct IType {
    /// The major opcode.
    pub opcode: u32,
    /// The minor opcode.
    pub funct3: u32,
    /// The destination.
    pub rd: RegIdx,
    /// The source.
    pub rs1: RegIdx,
    /// The 12-bit immediate; higher bits are dropped.
    pub imm: i32,
}

impl IType {
    /// The instruction word.
    #[must_use]
    pub const fn encode(self) -> u32 {
        (self.imm.cast_unsigned() & 0xFFF) << 20
            | reg(self.rs1) << 15
            | (self.funct3 & 0x7) << 12
            | reg(self.rd) << 7
            | (self.opcode & 0x7F)
    }
}

/// An S-type instruction: `imm[11:5] | rs2 | rs1 | funct3 | imm[4:0] | opcode`.
#[derive(Clone, Copy, Debug)]
pub struct SType {
    /// The major opcode.
    pub opcode: u32,
    /// The minor opcode.
    pub funct3: u32,
    /// The base address register.
    pub rs1: RegIdx,
    /// The register stored.
    pub rs2: RegIdx,
    /// The 12-bit offset.
    pub imm: i32,
}

impl SType {
    /// The instruction word.
    #[must_use]
    pub const fn encode(self) -> u32 {
        let imm = self.imm.cast_unsigned();
        ((imm >> 5) & 0x7F) << 25
            | reg(self.rs2) << 20
            | reg(self.rs1) << 15
            | (self.funct3 & 0x7) << 12
            | (imm & 0x1F) << 7
            | (self.opcode & 0x7F)
    }
}

/// A B-type instruction: a branch by a 13-bit even offset.
#[derive(Clone, Copy, Debug)]
pub struct BType {
    /// The major opcode.
    pub opcode: u32,
    /// The comparison.
    pub funct3: u32,
    /// The first source.
    pub rs1: RegIdx,
    /// The second source.
    pub rs2: RegIdx,
    /// The offset from the branch; bit 0 is dropped.
    pub offset: i32,
}

impl BType {
    /// The instruction word.
    #[must_use]
    pub const fn encode(self) -> u32 {
        let offset = self.offset.cast_unsigned();
        ((offset >> 12) & 1) << 31
            | ((offset >> 5) & 0x3F) << 25
            | reg(self.rs2) << 20
            | reg(self.rs1) << 15
            | (self.funct3 & 0x7) << 12
            | ((offset >> 1) & 0xF) << 8
            | ((offset >> 11) & 1) << 7
            | (self.opcode & 0x7F)
    }
}

/// A U-type instruction: `imm[31:12] | rd | opcode`.
#[derive(Clone, Copy, Debug)]
pub struct UType {
    /// The major opcode.
    pub opcode: u32,
    /// The destination.
    pub rd: RegIdx,
    /// The upper 20 bits of the immediate.
    pub imm20: u32,
}

impl UType {
    /// The instruction word.
    #[must_use]
    pub const fn encode(self) -> u32 {
        (self.imm20 & 0xF_FFFF) << 12 | reg(self.rd) << 7 | (self.opcode & 0x7F)
    }
}

/// A J-type instruction: a jump by a 21-bit even offset.
#[derive(Clone, Copy, Debug)]
pub struct JType {
    /// The major opcode.
    pub opcode: u32,
    /// The link register.
    pub rd: RegIdx,
    /// The offset from the jump; bit 0 is dropped.
    pub offset: i32,
}

impl JType {
    /// The instruction word.
    #[must_use]
    pub const fn encode(self) -> u32 {
        let offset = self.offset.cast_unsigned();
        ((offset >> 20) & 1) << 31
            | ((offset >> 1) & 0x3FF) << 21
            | ((offset >> 11) & 1) << 20
            | ((offset >> 12) & 0xFF) << 12
            | reg(self.rd) << 7
            | (self.opcode & 0x7F)
    }
}

/// A register field's value.
const fn reg(idx: RegIdx) -> u32 {
    idx.as_u8() as u32
}
