//! RISC-V Bit-Manipulation Extension (B).
//!
//! The 'B' extension is composed of four sub-extensions:
//! - [`Zba`]: Address generation (sh*add, add.uw, slli.uw)
//! - [`Zbb`]: Basic bit manipulation (clz, ctz, cpop, rotates, byte-reversal, etc.)
//! - [`Zbc`]: Carry-less multiplication (clmul, clmulh, clmulr)
//! - [`Zbs`]: Single-bit operations (bclr, bext, binv, bset)
//!
//! These instructions share the `OP_REG`, `OP_REG_32`, `OP_IMM`, and `OP_IMM_32`
//! opcodes with the base integer and M extensions, differentiated by their
//! `funct7`/`funct3` combinations.
//!
//! # Structure
//!
//! - `funct7`: Extension selectors for R-type B-extension instructions.
//! - `funct3`: Function codes identifying specific operations.

/// Function code 7 definitions for B-extension instructions.
pub mod funct7;

/// Function code 3 definitions for B-extension instructions.
pub mod funct3;
