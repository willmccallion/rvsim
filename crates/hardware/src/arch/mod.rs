//! Architectural state and the rules for changing it.
//!
//! What a hart holds (registers, CSRs, privilege, PMP) and what the
//! privileged spec says happens to it, independent of any pipeline.

/// Control and status registers.
pub mod csr;

/// Physical memory protection.
pub mod pmp;

/// Integer, floating-point and vector register files.
pub mod regs;

/// LR/SC reservation records.
pub mod reservation;

/// Address translation results and page-table updates.
pub mod translation;

/// Trap entry and return.
pub mod trap;

mod hart;
mod trigger;

pub use hart::{Hart, HartInit};
