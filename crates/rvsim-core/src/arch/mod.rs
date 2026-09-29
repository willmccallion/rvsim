//! Architectural state and the rules for changing it.
//!
//! What a hart holds (registers, CSRs, privilege, PMP) and what the
//! privileged spec says happens to it, independent of any pipeline.

pub mod csr;

pub mod pmp;

pub mod regs;

pub mod reservation;

pub mod translation;

pub mod trap;

mod hart;
mod trigger;

pub use hart::{Hart, HartInit};
