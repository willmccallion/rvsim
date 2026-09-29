//! System-on-Chip (SoC) Components.
//!
//! The bus, the coherence fabric, the memory controllers and the MMIO
//! devices.
//!
//! The CPU (`crate::system::SimState`) owns instances of them directly. There is
//! no aggregate `Soc` struct — the fields are flat on `SimState`.

/// System bus: routes requests to MMIO devices and the memory controller.
pub mod bus;

/// Cache coherence: protocol, home agent, interconnect, fabric.
pub mod coherence;

/// Memory-mapped I/O device implementations.
pub mod devices;

/// Memory controller implementations.
pub mod memory;

/// Device trait definitions for MMIO access.
pub mod traits;

use crate::sim::components::CacheId;

/// `CacheId` of the shared LLC in single-core configurations.
///
/// Convention: a core occupies `CacheId`s `[core_base, core_base+3)`
/// (L1I, L1D, L2). The shared LLC sits immediately after the last core's
/// caches. For a single core that places it at `CacheId(3)`.
pub const L3_CACHE_ID: CacheId = CacheId::new(3);
