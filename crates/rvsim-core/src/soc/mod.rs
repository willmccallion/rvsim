//! System-on-chip components.
//!
//! The bus, the caches, the coherence fabric, the memory controllers and
//! the MMIO devices.
//!
//! The CPU (`crate::system::SystemState`) owns instances of them directly. There is
//! no aggregate `Soc` struct — the fields are flat on `SystemState`.

pub mod bus;

pub mod cache;

pub mod coherence;

pub mod devices;

pub mod memory;

pub mod traits;

use crate::sim::components::CacheId;

/// `CacheId` of the shared LLC in single-core configurations.
///
/// Convention: a core occupies `CacheId`s `[core_base, core_base+3)`
/// (L1I, L1D, L2). The shared LLC sits immediately after the last core's
/// caches. For a single core that places it at `CacheId(3)`.
pub const L3_CACHE_ID: CacheId = CacheId::new(3);
