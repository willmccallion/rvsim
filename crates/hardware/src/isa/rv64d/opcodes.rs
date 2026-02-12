//! RISC-V Double-Precision (D) Opcodes.
//!
//! The 'D' extension shares the major opcodes with the 'F' extension.
//! These constants are re-exported or defined here for clarity when implementing
//! double-precision logic.

/// Floating-point Load (FLD).
pub const OP_LOAD_FP: u32 = 0b0000111;

/// Floating-point Store (FSD).
pub const OP_STORE_FP: u32 = 0b0100111;

/// Floating-point Arithmetic (FADD.D, FSUB.D, etc.).
pub const OP_FP: u32 = 0b1010011;

/// Fused Multiply-Add (FMADD.D).
pub const OP_FMADD: u32 = 0b1000011;

/// Fused Multiply-Subtract (FMSUB.D).
pub const OP_FMSUB: u32 = 0b1000111;

/// Fused Negated Multiply-Subtract (FNMSUB.D).
pub const OP_FNMSUB: u32 = 0b1001011;

/// Fused Negated Multiply-Add (FNMADD.D).
pub const OP_FNMADD: u32 = 0b1001111;
