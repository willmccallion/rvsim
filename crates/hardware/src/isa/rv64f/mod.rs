//! RISC-V Single-Precision Floating-Point Extension (F).
//!
//! Defines instructions for single-precision (32-bit) floating-point arithmetic.
//!
//! # Structure
//!
//! - `opcodes`: Major opcodes for floating-point load, store, and arithmetic.
//! - `funct3`: Function codes for rounding modes and comparison types.
//! - `funct7`: Function codes for specific arithmetic operations.

/// Function code 3 definitions for single-precision operations.
pub mod funct3;

/// Function code 7 definitions for single-precision operations.
pub mod funct7;

/// Single-precision floating-point opcodes.
pub mod opcodes;
