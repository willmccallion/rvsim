//! RISC-V Single-Precision Floating-Point Extension (F).
//!
//! Defines instructions for single-precision (32-bit) floating-point arithmetic.
//!
//! # Structure
//!
//! - `opcodes`: Major opcodes for floating-point load, store, and arithmetic.
//! - `funct3`: Function codes for rounding modes and comparison types.
//! - `funct7`: Function codes for specific arithmetic operations.

pub mod funct3;

pub mod funct7;

pub mod opcodes;
