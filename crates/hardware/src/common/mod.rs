//! Common utilities and types shared across the simulator.

/// Address type definitions (physical and virtual addresses).
pub mod addr;

/// Common constants used throughout the simulator.
pub mod constants;

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

pub use addr::{Asid, IrqId, LineAddr, PhysAddr, Ppn, VirtAddr, Vpn};
pub use constants::{PAGE_SHIFT, VPN_MASK};
pub use data::AccessType;
pub use error::{
    DirtyUpdates, ExceptionStage, LrScRecord, PteUpdate, SfenceVmaInfo, TranslationResult, Trap,
};
pub use ids::{CoreId, HartId, InstSeq};
pub use reg::RegisterFile;
pub use sim_error::SimError;
