//! System-on-Chip (SoC) Components.
//!
//! This module organizes the components that make up the simulated system,
//! including the system bus, memory controllers, devices, and the builder
//! for assembling the system.

/// System builder for assembling SoC components.
pub mod builder;

/// Memory-mapped I/O device implementations.
pub mod devices;

/// System bus interconnect and routing.
pub mod interconnect;

/// Memory controller implementations.
pub mod memory;

/// Device trait definitions for MMIO access.
pub mod traits;

pub use builder::System;
