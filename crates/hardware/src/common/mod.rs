//! Common utilities and types shared across the simulator.

/// Address type definitions (physical and virtual addresses).
pub mod addr;

/// Memory access type definitions.
pub mod data;

/// Error types and trap definitions.
pub mod error;

/// Register file implementation.
pub mod reg;

/// Top-level simulator error type.
pub mod sim_error;

/// Hart and physical-core identifier newtypes.
pub mod ids;

pub use addr::{
    Asid, IrqId, LineAddr, PAGE_OFFSET_MASK, PAGE_SHIFT, PhysAddr, Ppn, VPN_MASK, VirtAddr, Vpn,
};
pub use data::AccessType;
pub use error::{
    DirtyUpdates, ExceptionStage, LrScRecord, PteUpdate, SfenceVmaInfo, TranslationResult,
};
pub use ids::{CoreId, HartId, InstSeq};
pub use reg::RegisterFile;
pub use sim_error::SimError;
