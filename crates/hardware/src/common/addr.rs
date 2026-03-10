//! Physical and Virtual Address types.
//!
//! This module defines strong types for physical and virtual addresses to prevent
//! accidental mixing of address spaces. It provides the following:
//! 1. **Type Safety:** Distinguishes between virtual and physical address spaces at compile time.
//! 2. **Address Manipulation:** Provides helper methods for extracting page offsets and raw values.
//! 3. **MMU Integration:** Acts as the primary interface for memory translation operations.

/// An Address Space Identifier (ASID) from SATP[59:44].
///
/// Used by the TLB to distinguish translations belonging to different address spaces,
/// enabling OS context switches without a full TLB flush.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Asid(u16);

impl Asid {
    /// Creates a new ASID from a raw 16-bit value.
    #[inline(always)]
    pub const fn new(val: u16) -> Self {
        Self(val)
    }

    /// Returns the raw 16-bit value.
    #[inline(always)]
    pub const fn val(self) -> u16 {
        self.0
    }
}

/// An Interrupt Request Identifier for PLIC interrupt lines.
///
/// Represents a hardware interrupt source number (1–1023); 0 is reserved/no interrupt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct IrqId(u32);

impl IrqId {
    /// Creates a new `IrqId` from a raw 32-bit value.
    #[inline(always)]
    pub const fn new(val: u32) -> Self {
        Self(val)
    }

    /// Returns the raw 32-bit value.
    #[inline(always)]
    pub const fn val(self) -> u32 {
        self.0
    }
}

/// A Virtual Page Number in the RISC-V SV39 address space.
///
/// Represents the upper 27 bits of a 39-bit virtual address (bits 38:12),
/// used as a TLB tag and page table index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Vpn(u64);

/// A Physical Page Number in the RISC-V address space.
///
/// Represents the upper bits of a physical address (bits 55:12),
/// used as TLB data and in page table entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Ppn(u64);

impl Vpn {
    /// Creates a new VPN from a raw 64-bit value.
    #[inline(always)]
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Returns the raw 64-bit value.
    #[inline(always)]
    pub const fn val(self) -> u64 {
        self.0
    }

    /// Converts this VPN to a physical address by shifting left by `PAGE_SHIFT`.
    #[inline(always)]
    pub const fn to_addr(self) -> u64 {
        self.0 << 12
    }
}

impl Ppn {
    /// Creates a new PPN from a raw 64-bit value.
    #[inline(always)]
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Returns the raw 64-bit value.
    #[inline(always)]
    pub const fn val(self) -> u64 {
        self.0
    }

    /// Converts this PPN to a physical address by shifting left by `PAGE_SHIFT`.
    #[inline(always)]
    pub const fn to_addr(self) -> u64 {
        self.0 << 12
    }
}

/// A virtual address in the RISC-V address space.
///
/// Virtual addresses are used by software and must be translated to physical addresses
/// through the Memory Management Unit (MMU) before accessing memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct VirtAddr(pub u64);

/// A physical address in the RISC-V address space.
///
/// Physical addresses represent actual hardware memory locations and are used
/// after virtual-to-physical address translation has completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
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
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    /// Returns the raw 64-bit address value.
    ///
    /// # Returns
    ///
    /// The underlying 64-bit address value.
    #[inline(always)]
    pub const fn val(&self) -> u64 {
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
    pub const fn page_offset(&self) -> u64 {
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
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    /// Returns the raw 64-bit address value.
    ///
    /// # Returns
    ///
    /// The underlying 64-bit address value.
    #[inline(always)]
    pub const fn val(&self) -> u64 {
        self.0
    }
}
