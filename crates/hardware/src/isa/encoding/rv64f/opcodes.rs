//! RISC-V Floating-Point (F/D) Opcodes.
//!
//! Defines the major opcodes for floating-point loads, stores, and
//! arithmetic operations.

/// Floating-point Load (FLW, FLD).
pub const OP_LOAD_FP: u32 = 0b0000111;

/// Floating-point Store (FSW, FSD).
pub const OP_STORE_FP: u32 = 0b0100111;

/// Floating-point Arithmetic (FADD, FSUB, etc.).
pub const OP_FP: u32 = 0b1010011;

/// Fused Multiply-Add (FMADD).
pub const OP_FMADD: u32 = 0b1000011;

/// Fused Multiply-Subtract (FMSUB).
pub const OP_FMSUB: u32 = 0b1000111;

/// Fused Negated Multiply-Subtract (FNMSUB).
pub const OP_FNMSUB: u32 = 0b1001011;

/// Fused Negated Multiply-Add (FNMADD).
pub const OP_FNMADD: u32 = 0b1001111;
