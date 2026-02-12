//! Instruction encoding and decoding utilities.
//!
//! Provides bit extraction functions and structures for decoding
//! RISC-V instruction fields from 32-bit instruction encodings.

/// Bit mask for extracting the opcode field (bits 0-6).
pub const OPCODE_MASK: u32 = 0x7F;
/// Bit mask for extracting the destination register field (bits 7-11).
pub const RD_MASK: u32 = 0x1F;
/// Bit mask for extracting the first source register field (bits 15-19).
pub const RS1_MASK: u32 = 0x1F;
/// Bit mask for extracting the second source register field (bits 20-24).
pub const RS2_MASK: u32 = 0x1F;
/// Bit mask for extracting the funct3 field (bits 12-14).
pub const FUNCT3_MASK: u32 = 0x7;
/// Bit mask for extracting the funct7 field (bits 25-31).
pub const FUNCT7_MASK: u32 = 0x7F;
/// Bit mask for extracting the CSR address field (bits 20-31).
pub const CSR_MASK: u32 = 0xFFF;

/// Trait for extracting instruction fields from encoded instructions.
///
/// Provides methods to extract all standard RISC-V instruction fields
/// from a 32-bit instruction encoding.
pub trait InstructionBits {
    /// Extracts the opcode field (bits 0-6).
    ///
    /// The opcode determines the instruction format and operation category.
    /// Returns the 7-bit opcode value.
    fn opcode(&self) -> u32;

    /// Extracts the destination register field (bits 7-11).
    ///
    /// Returns the 5-bit register index (0-31) for the destination register.
    /// Register 0 (x0) is hardwired to zero and writes are ignored.
    fn rd(&self) -> usize;

    /// Extracts the first source register field (bits 15-19).
    ///
    /// Returns the 5-bit register index (0-31) for the first source operand.
    fn rs1(&self) -> usize;

    /// Extracts the second source register field (bits 20-24).
    ///
    /// Returns the 5-bit register index (0-31) for the second source operand.
    fn rs2(&self) -> usize;

    /// Extracts the funct3 field (bits 12-14).
    ///
    /// Used to distinguish between different operations within the same opcode.
    /// Returns the 3-bit funct3 value.
    fn funct3(&self) -> u32;

    /// Extracts the funct7 field (bits 25-31).
    ///
    /// Used for RV64 operations and to distinguish between standard and
    /// alternate encodings (e.g., ADD vs SUB). Returns the 7-bit funct7 value.
    fn funct7(&self) -> u32;

    /// Extracts the CSR address field (bits 20-31).
    ///
    /// Returns the 12-bit CSR address used for CSR read/write operations.
    fn csr(&self) -> u32;
    /// Extracts the third source register field (bits 27-31, for FMA instructions).
    ///
    /// # Returns
    ///
    /// The 5-bit register index (0-31).
    fn rs3(&self) -> usize;
}

impl InstructionBits for u32 {
    /// Extracts the opcode field (bits 0-6) using bitwise AND with OPCODE_MASK.
    ///
    /// The opcode determines the instruction format and operation category.
    /// This is the first field decoded and drives all subsequent field extraction.
    #[inline(always)]
    fn opcode(&self) -> u32 {
        self & OPCODE_MASK
    }

    /// Extracts the destination register field (bits 7-11).
    ///
    /// Shifts right by 7 bits to align the register field, then masks to extract
    /// the 5-bit register index. Register 0 (x0) is hardwired to zero.
    #[inline(always)]
    fn rd(&self) -> usize {
        ((self >> 7) & RD_MASK) as usize
    }

    /// Extracts the first source register field (bits 15-19).
    ///
    /// Shifts right by 15 bits to align the register field, then masks to extract
    /// the 5-bit register index for the first source operand.
    #[inline(always)]
    fn rs1(&self) -> usize {
        ((self >> 15) & RS1_MASK) as usize
    }

    /// Extracts the second source register field (bits 20-24).
    ///
    /// Shifts right by 20 bits to align the register field, then masks to extract
    /// the 5-bit register index for the second source operand.
    #[inline(always)]
    fn rs2(&self) -> usize {
        ((self >> 20) & RS2_MASK) as usize
    }

    /// Extracts the third source register field (bits 27-31).
    ///
    /// Used for fused multiply-add (FMA) instructions that require three source
    /// operands. Shifts right by 27 bits and masks to extract the 5-bit register index.
    #[inline(always)]
    fn rs3(&self) -> usize {
        ((self >> 27) & RS1_MASK) as usize
    }

    /// Extracts the funct3 field (bits 12-14).
    ///
    /// Shifts right by 12 bits and masks to extract the 3-bit function code.
    /// Used to distinguish between different operations within the same opcode
    /// (e.g., ADD vs SUB, BEQ vs BNE).
    #[inline(always)]
    fn funct3(&self) -> u32 {
        (self >> 12) & FUNCT3_MASK
    }

    /// Extracts the funct7 field (bits 25-31).
    ///
    /// Shifts right by 25 bits and masks to extract the 7-bit function code.
    /// Used for RV64 operations and to distinguish between standard and alternate
    /// encodings (e.g., ADD vs SUB when bit 5 is set).
    #[inline(always)]
    fn funct7(&self) -> u32 {
        (self >> 25) & FUNCT7_MASK
    }

    /// Extracts the CSR address field (bits 20-31).
    ///
    /// Shifts right by 20 bits and masks to extract the 12-bit CSR address.
    /// Used for Control and Status Register read/write operations. The CSR address
    /// space is 4K entries, allowing access to all standard and custom CSRs.
    #[inline(always)]
    fn csr(&self) -> u32 {
        (self >> 20) & CSR_MASK
    }
}

/// Decoded instruction structure containing all extracted fields.
///
/// Contains all instruction fields extracted during decoding, including
/// opcode, register indices, function codes, and sign-extended immediate.
#[derive(Clone, Debug, Default)]
pub struct Decoded {
    /// Raw 32-bit instruction encoding.
    pub raw: u32,
    /// Extracted opcode field.
    pub opcode: u32,
    /// Destination register index.
    pub rd: usize,
    /// First source register index.
    pub rs1: usize,
    /// Second source register index.
    pub rs2: usize,
    /// Function code field 3.
    pub funct3: u32,
    /// Function code field 7.
    pub funct7: u32,
    /// Sign-extended immediate value.
    pub imm: i64,
}
