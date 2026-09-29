//! Memory-Mapped IO Devices.
//!
//! This module contains implementations of various hardware devices
//! found in the system-on-chip, such as timers (CLINT), interrupt controllers (PLIC),
//! serial ports (UART), and block devices (virtio).

pub mod clint;

pub mod goldfish_rtc;

pub mod htif;

pub mod plic;

pub mod sim_control;

pub mod syscon;

pub mod uart;

pub mod virtio_disk;

pub use clint::Clint;
pub use goldfish_rtc::GoldfishRtc;
pub use htif::Htif;
pub use plic::Plic;
pub use sim_control::{SimControl, SimOp};
pub use syscon::SysCon;
pub use uart::Uart;
pub use virtio_disk::VirtioBlock;

pub use crate::soc::traits::Device;
