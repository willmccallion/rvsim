//! # Architectural Components
//!
//! This module provides the core architectural building blocks for the RISC-V implementation.
//! It encompasses register files, execution state, and specific architectural rules
//! such as floating-point NaN-boxing.

/// Unit tests for Floating-Point Register (FPR) functionality and NaN-boxing.
///
/// This module verifies the correct behavior of the 32 floating-point registers,
/// ensuring proper storage of 64-bit values and compliance with RISC-V
/// NaN-boxing requirements for 32-bit values.
pub mod fpr_nan_boxing;

/// Unit tests for RISC-V privilege mode conversions and representations.
///
/// This module verifies the correct behavior of privilege mode conversions,
/// ordering, and display formatting.
pub mod mode;

/// Unit tests for General-Purpose Register file.
///
/// This module verifies the functionality of the GPR file, including
/// register reads/writes and the hardwired zero in x0.
pub mod gpr;
