//! Privileged architecture: privilege modes, traps, and trap cause codes.

/// Exception and interrupt cause code definitions.
pub mod cause;

mod mode;
mod trap;

pub use mode::PrivilegeMode;
pub use trap::Trap;
