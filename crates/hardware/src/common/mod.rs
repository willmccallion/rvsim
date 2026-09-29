//! Common utilities and types shared across the simulator.

/// Kinds of memory access (fetch, load, store).
pub mod access;

/// Address type definitions (physical and virtual addresses).
pub mod addr;

/// Top-level simulator error type.
pub mod error;

/// Hart and physical-core identifier newtypes.
pub mod ids;

pub use access::AccessType;
pub use addr::{
    Asid, IrqId, LineAddr, PAGE_OFFSET_MASK, PAGE_SHIFT, PhysAddr, Ppn, VPN_MASK, VirtAddr, Vpn,
    crosses_cache_line,
};
pub use error::SimError;
pub use ids::{CoreId, HartId, InstSeq};
