//! RISC-V Double-Precision Floating-Point Extension (D).
//!
//! The 'D' extension adds 64-bit floating-point registers and operations.
//! It generally shares major opcodes with the 'F' extension but uses specific
//! format fields or function codes to designate double-precision operations.
//!
//! # Structure
//!
//! - `opcodes`: Shared floating-point opcodes.
//! - `funct7`: Format-specific operation codes.

pub mod funct7;

pub mod opcodes;
