//! System-on-chip components.
//!
//! The bus, the caches, the coherence fabric, the memory controllers and
//! the MMIO devices.
//!
//! The [`uncore::Uncore`] owns them; `crate::system::SystemState` pairs it
//! with the harts and cores.

pub mod bus;

pub mod cache;

pub mod coherence;

pub mod devices;

pub mod memory;

pub mod topology;

pub mod traits;

pub mod uncore;
