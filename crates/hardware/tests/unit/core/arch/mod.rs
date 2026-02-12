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
