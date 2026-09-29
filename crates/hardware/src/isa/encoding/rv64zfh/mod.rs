//! RISC-V Half-Precision Floating-Point Extension (Zfh).
//!
//! Defines instructions for half-precision (16-bit) floating-point arithmetic.
//! Half-precision values live in `f` registers NaN-boxed into 64 bits (upper
//! 48 bits all ones).

/// Function code 7 definitions for half-precision operations.
pub mod funct7;
