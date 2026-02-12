//! Memory-Mapped IO Devices.
//!
//! This module contains implementations of various hardware devices
//! found in the SoC, such as timers (CLINT), interrupt controllers (PLIC),
//! serial ports (UART), and block devices (VirtIO).

/// Core Local Interruptor (timer and software interrupt controller).
pub mod clint;

/// Goldfish RTC (Real-Time Clock) device.
pub mod goldfish_rtc;

/// Platform-Level Interrupt Controller (PLIC).
pub mod plic;

/// System Controller (power and reset control).
pub mod syscon;

/// UART 16550-compatible serial port.
pub mod uart;

/// VirtIO block device (disk emulation).
pub mod virtio_disk;

pub use clint::Clint;
pub use goldfish_rtc::GoldfishRtc;
pub use plic::Plic;
pub use syscon::SysCon;
pub use uart::Uart;
pub use virtio_disk::VirtioBlock;

pub use crate::soc::traits::Device;
