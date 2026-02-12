//! Python bindings for SoC devices.
//!
//! This module exposes placeholder types for device introspection from Python:
//! 1. **UART:** Serial port device (actual I/O in core simulator).
//! 2. **PLIC:** Platform-Level Interrupt Controller (IRQ state in core).
//! 3. **VirtIO block:** Block device (disk I/O in core).

/// PLIC (Platform-Level Interrupt Controller) binding.
pub mod plic;
/// UART device binding.
pub mod uart;
/// VirtIO block device binding.
pub mod virtio;
