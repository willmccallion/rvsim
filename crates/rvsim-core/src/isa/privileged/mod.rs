//! Privileged architecture: privilege modes, paging modes, traps, and
//! trap cause codes.

pub mod cause;

mod mode;
mod paging;
mod trap;

pub use mode::PrivilegeMode;
pub use paging::PagingMode;
pub use trap::Trap;
