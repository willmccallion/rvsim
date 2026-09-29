//! RISC-V Vector Extension (RVV 1.0) ISA definitions.

/// Vector arithmetic opcode.
pub mod opcodes;

/// Vector arithmetic funct3 categories.
pub mod funct3;

/// Vector arithmetic funct6 operation codes.
pub mod funct6;

/// Vector-specific instruction field extraction.
pub mod encoding;
