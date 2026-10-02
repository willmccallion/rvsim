//! # Architectural Components
//!
//! This module provides the core architectural building blocks for the RISC-V implementation.
//! It encompasses register files, execution state, and specific architectural rules
//! such as floating-point NaN-boxing.

pub mod csr;
pub mod fpr_nan_boxing;
pub mod gpr;
pub mod hart;
pub mod mode;
pub mod pmp;
