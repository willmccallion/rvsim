//! System Controller (SysCon).
//!
//! A simple memory-mapped device used to control system power and reset states.
//! It is primarily used by the kernel or test environment to gracefully exit
//! the simulation or trigger a reset.
//!
//! # Registers
//!
//! * `0x00`: Command Register (Write Only)
//!   * `0x5555`: Power Off
//!   * `0x7777`: Reset
//!   * `0x3333`: Failure/Panic

use crate::soc::devices::Device;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// SysCon device structure.
pub struct SysCon {
    /// Base physical address of the device.
    base_addr: u64,
    /// Shared atomic flag to signal the simulation loop to exit.
    exit_signal: Arc<AtomicU64>,
}

impl SysCon {
    /// Creates a new SysCon device.
    ///
    /// # Arguments
    ///
    /// * `base_addr` - The base physical address.
    /// * `exit_signal` - Shared atomic for signaling exit codes.
    pub fn new(base_addr: u64, exit_signal: Arc<AtomicU64>) -> Self {
        Self {
            base_addr,
            exit_signal,
        }
    }
}

impl Device for SysCon {
    /// Returns the device name.
    fn name(&self) -> &str {
        "SysCon"
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
    /// Reads a word (unimplemented, returns 0).
    fn read_u32(&mut self, _offset: u64) -> u32 {
        0
    }
    /// Reads a double-word (unimplemented, returns 0).
    fn read_u64(&mut self, _offset: u64) -> u64 {
        0
    }

    /// Writes a byte (unimplemented).
    fn write_u8(&mut self, _offset: u64, _val: u8) {}
    /// Writes a half-word (unimplemented).
    fn write_u16(&mut self, _offset: u64, _val: u16) {}

    /// Writes a word (32-bit) to the device.
    ///
    /// Interprets specific magic values to trigger system events.
    fn write_u32(&mut self, offset: u64, val: u32) {
        if offset == 0 {
            match val {
                0x5555 => {
                    println!("[SysCon] Poweroff signal received.");
                    self.exit_signal.store(0, Ordering::Relaxed)
                }
                0x7777 => {
                    println!("[SysCon] Reset signal received (Simulated as Exit).");
                    self.exit_signal.store(0, Ordering::Relaxed)
                }
                0x3333 => {
                    println!("[SysCon] Failure signal received.");
                    self.exit_signal.store(1, Ordering::Relaxed)
                }
                _ => {}
            }
        }
    }

    /// Writes a double-word (delegates to write_u32).
    fn write_u64(&mut self, offset: u64, val: u64) {
        self.write_u32(offset, val as u32);
    }
}
