//! Common utilities and types shared across the simulator.

pub mod access;

pub mod addr;

pub mod error;

pub mod ids;

pub mod trace;

pub use access::AccessType;
pub use addr::{
    Asid, IrqId, LineAddr, PAGE_OFFSET_MASK, PAGE_SHIFT, PhysAddr, Ppn, VPN_MASK, VirtAddr, Vpn,
    crosses_cache_line,
};
pub use error::SimError;
pub use ids::{CoreId, HartId, InstSeq};
