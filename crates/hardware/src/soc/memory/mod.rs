//! Physical System Memory (DRAM).
//!
//! This module implements the main system memory device. It provides:
//! 1. **Buffer:** Backing storage (e.g., `DramBuffer`) for RAM contents.
//! 2. **Memory:** Device implementation that maps the buffer at a physical base address.
//! 3. **Controller:** Latency modeling (simple or DRAM row-buffer) for timing simulation.

/// DRAM buffer implementation (e.g., mmap or `Vec`) for raw byte storage.
pub mod buffer;

/// Memory controller implementations for access latency modeling.
pub mod controller;

use self::buffer::DramBuffer;
use crate::soc::devices::Device;
use std::sync::Arc;

/// System Memory structure.
pub struct Memory {
    /// Shared reference to the underlying memory buffer.
    buffer: Arc<DramBuffer>,
    /// The base physical address where this memory is mapped.
    base_addr: u64,
}

impl Memory {
    /// Creates a new Memory instance.
    ///
    /// # Arguments
    ///
    /// * `buffer` - Shared DRAM buffer.
    /// * `base_addr` - Starting physical address.
    pub fn new(buffer: Arc<DramBuffer>, base_addr: u64) -> Self {
        Self { buffer, base_addr }
    }

    /// Loads a byte slice into memory at a specific offset.
    ///
    /// Used for loading kernels, disk images, or other binaries during system setup.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to write.
    /// * `offset` - The byte offset relative to the memory base address.
    pub fn load(&mut self, data: &[u8], offset: usize) {
        if offset + data.len() <= self.buffer.len() {
            self.buffer.write_slice(offset, data);
        }
    }

    /// Returns a raw mutable pointer to the underlying memory buffer.
    ///
    /// Required for devices like VirtIO that perform direct memory access (DMA)
    /// using raw pointers for performance or FFI compatibility.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.buffer.as_mut_ptr()
    }
}

impl Device for Memory {
    /// Returns the device name.
    fn name(&self) -> &str {
        "DRAM"
    }

    /// Returns the address range (Base, Size).
    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, self.buffer.len() as u64)
    }

    /// Reads a byte from memory.
    fn read_u8(&mut self, offset: u64) -> u8 {
        self.buffer.read_u8(offset as usize)
    }

    /// Reads a half-word (16-bit) from memory (Little Endian).
    fn read_u16(&mut self, offset: u64) -> u16 {
        let i = offset as usize;
        let slice = self.buffer.read_slice(i, 2);
        u16::from_le_bytes(slice.try_into().unwrap())
    }

    /// Reads a word (32-bit) from memory (Little Endian).
    fn read_u32(&mut self, offset: u64) -> u32 {
        let i = offset as usize;
        let slice = self.buffer.read_slice(i, 4);
        u32::from_le_bytes(slice.try_into().unwrap())
    }

    /// Reads a double-word (64-bit) from memory (Little Endian).
    fn read_u64(&mut self, offset: u64) -> u64 {
        let i = offset as usize;
        let slice = self.buffer.read_slice(i, 8);
        u64::from_le_bytes(slice.try_into().unwrap())
    }

    /// Writes a byte to memory.
    fn write_u8(&mut self, offset: u64, val: u8) {
        self.buffer.write_u8(offset as usize, val);
    }

    /// Writes a half-word to memory (Little Endian).
    fn write_u16(&mut self, offset: u64, val: u16) {
        self.buffer.write_slice(offset as usize, &val.to_le_bytes());
    }

    /// Writes a word to memory (Little Endian).
    fn write_u32(&mut self, offset: u64, val: u32) {
        self.buffer.write_slice(offset as usize, &val.to_le_bytes());
    }

    /// Writes a double-word to memory (Little Endian).
    fn write_u64(&mut self, offset: u64, val: u64) {
        self.buffer.write_slice(offset as usize, &val.to_le_bytes());
    }

    /// Writes a slice of bytes to memory.
    fn write_bytes(&mut self, offset: u64, data: &[u8]) {
        self.load(data, offset as usize);
    }

    /// Downcasts the device to a mutable Memory reference.
    fn as_memory_mut(&mut self) -> Option<&mut Memory> {
        Some(self)
    }
}
