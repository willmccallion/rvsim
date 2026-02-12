//! Device trait for memory-mapped I/O.
//!
//! This module defines the `Device` trait implemented by all bus-attached components. It provides:
//! 1. **Identification:** `name` and `address_range` for bus routing.
//! 2. **Access:** Byte, half, word, and doubleword read/write at device-relative offsets.
//! 3. **Lifecycle:** Optional `tick` and IRQ reporting for timer and interrupt devices.
//! 4. **Downcasting:** Optional casts to `Plic`, `Uart`, or `Memory` for device-specific access.
//!
//! All implementors must be `Send + Sync` for use with the Python bindings and multi-threaded simulation.

use crate::soc::devices::{Plic, Uart};
use crate::soc::memory::Memory;

/// Trait for memory-mapped I/O devices attached to the system bus.
///
/// Devices provide a name, address range, and read/write methods. Optional methods support
/// ticking (e.g., timers), IRQ reporting, and type-specific access (Plic, Uart, Memory).
pub trait Device: Send + Sync {
    /// Returns a short name for this device (e.g., `"UART0"`, `"DRAM"`).
    fn name(&self) -> &str;
    /// Returns (base_address, size_in_bytes) for this device's MMIO or memory region.
    fn address_range(&self) -> (u64, u64);
    /// Reads one byte at the given device-relative offset.
    fn read_u8(&mut self, offset: u64) -> u8;
    /// Reads two bytes (little-endian) at the given offset.
    fn read_u16(&mut self, offset: u64) -> u16;
    /// Reads four bytes (little-endian) at the given offset.
    fn read_u32(&mut self, offset: u64) -> u32;
    /// Reads eight bytes (little-endian) at the given offset.
    fn read_u64(&mut self, offset: u64) -> u64;
    /// Writes one byte at the given offset.
    fn write_u8(&mut self, offset: u64, val: u8);
    /// Writes two bytes (little-endian) at the given offset.
    fn write_u16(&mut self, offset: u64, val: u16);
    /// Writes four bytes (little-endian) at the given offset.
    fn write_u32(&mut self, offset: u64, val: u32);
    /// Writes eight bytes (little-endian) at the given offset.
    fn write_u64(&mut self, offset: u64, val: u64);

    /// Writes a contiguous byte slice at the given offset (default: byte-by-byte).
    fn write_bytes(&mut self, offset: u64, data: &[u8]) {
        for (i, byte) in data.iter().enumerate() {
            self.write_u8(offset + i as u64, *byte);
        }
    }

    /// Advances device state by one cycle; returns `true` if an IRQ was raised (e.g., timer).
    fn tick(&mut self) -> bool {
        false
    }
    /// Returns the IRQ ID for this device if it can raise interrupts (e.g., PLIC line).
    fn get_irq_id(&self) -> Option<u32> {
        None
    }

    /// Returns a mutable reference as `Plic` if this device is the PLIC; otherwise `None`.
    fn as_plic_mut(&mut self) -> Option<&mut Plic> {
        None
    }
    /// Returns a mutable reference as `Uart` if this device is a UART; otherwise `None`.
    fn as_uart_mut(&mut self) -> Option<&mut Uart> {
        None
    }
    /// Returns a mutable reference as `Memory` if this device is RAM; otherwise `None`.
    fn as_memory_mut(&mut self) -> Option<&mut Memory> {
        None
    }
}
