//! Physical and Virtual Address types.
//!
//! This module defines strong types for physical and virtual addresses to prevent
//! accidental mixing of address spaces. It provides the following:
//! 1. **Type Safety:** Distinguishes between virtual and physical address spaces at compile time.
//! 2. **Address Manipulation:** Provides helper methods for extracting page offsets and raw values.
//! 3. **MMU Integration:** Acts as the primary interface for memory translation operations.

/// A virtual address in the RISC-V address space.
///
/// Virtual addresses are used by software and must be translated to physical addresses
/// through the Memory Management Unit (MMU) before accessing memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtAddr(pub u64);

/// A physical address in the RISC-V address space.
///
/// Physical addresses represent actual hardware memory locations and are used
/// after virtual-to-physical address translation has completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysAddr(pub u64);

impl VirtAddr {
    /// Creates a new virtual address from a raw 64-bit value.
    ///
    /// # Arguments
    ///
    /// * `addr` - The raw 64-bit address value.
    ///
    /// # Returns
    ///
    /// A new `VirtAddr` instance wrapping the provided address.
    #[inline(always)]
    pub fn new(addr: u64) -> Self {
        Self(addr)
    }

    /// Returns the raw 64-bit address value.
    ///
    /// # Returns
    ///
    /// The underlying 64-bit address value.
    #[inline(always)]
    pub fn val(&self) -> u64 {
        self.0
    }

    /// Extracts the page offset from the virtual address.
    ///
    /// The page offset is the lower 12 bits of the address, representing
    /// the byte offset within a 4KB page.
    ///
    /// # Returns
    ///
    /// The page offset (0-4095) as a `u64`.
    pub fn page_offset(&self) -> u64 {
        self.0 & 0xFFF
    }
}

impl PhysAddr {
    /// Creates a new physical address from a raw 64-bit value.
    ///
    /// # Arguments
    ///
    /// * `addr` - The raw 64-bit address value.
    ///
    /// # Returns
    ///
    /// A new `PhysAddr` instance wrapping the provided address.
    #[inline(always)]
    pub fn new(addr: u64) -> Self {
        Self(addr)
    }

    /// Returns the raw 64-bit address value.
    ///
    /// # Returns
    ///
    /// The underlying 64-bit address value.
    #[inline(always)]
    pub fn val(&self) -> u64 {
        self.0
    }
}
