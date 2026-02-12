//! Goldfish Real-Time Clock (RTC).
//!
//! A virtual RTC device commonly used in Android emulators (QEMU).
//! It provides the current system time in nanoseconds.
//!
//! # Memory Map
//!
//! * `0x00`: Time (Low 32 bits)
//! * `0x04`: Time (High 32 bits)

use crate::soc::devices::Device;
use std::time::{SystemTime, UNIX_EPOCH};

/// Goldfish RTC device structure.
pub struct GoldfishRtc {
    /// Base physical address of the device.
    base_addr: u64,
}

impl GoldfishRtc {
    /// Creates a new Goldfish RTC device.
    pub fn new(base_addr: u64) -> Self {
        Self { base_addr }
    }

    /// Retrieves the current system time in nanoseconds.
    fn get_time_ns(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    }
}

impl Device for GoldfishRtc {
    /// Returns the device name.
    fn name(&self) -> &str {
        "GoldfishRTC"
    }

    /// Returns the address range (Base, Size).
    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x1000)
    }

    /// Reads a byte (unimplemented, returns 0).
    fn read_u8(&mut self, _offset: u64) -> u8 {
        0
    }
    /// Reads a half-word (unimplemented, returns 0).
    fn read_u16(&mut self, _offset: u64) -> u16 {
        0
    }

    /// Reads a word (32-bit) from the device.
    ///
    /// Returns the lower or upper 32 bits of the current nanosecond timestamp.
    fn read_u32(&mut self, offset: u64) -> u32 {
        let time = self.get_time_ns();
        match offset {
            0x00 => time as u32,
            0x04 => (time >> 32) as u32,
            _ => 0,
        }
    }

    /// Reads a double-word (64-bit) from the device.
    ///
    /// Returns the full 64-bit nanosecond timestamp.
    fn read_u64(&mut self, offset: u64) -> u64 {
        let time = self.get_time_ns();
        match offset {
            0x00 => time,
            _ => 0,
        }
    }

    /// Writes a byte (unimplemented).
    fn write_u8(&mut self, _offset: u64, _val: u8) {}
    /// Writes a half-word (unimplemented).
    fn write_u16(&mut self, _offset: u64, _val: u16) {}
    /// Writes a word (unimplemented).
    fn write_u32(&mut self, _offset: u64, _val: u32) {}
    /// Writes a double-word (unimplemented).
    fn write_u64(&mut self, _offset: u64, _val: u64) {}

    /// Returns the Interrupt Request (IRQ) ID associated with this device.
    fn get_irq_id(&self) -> Option<u32> {
        Some(11)
    }
}
