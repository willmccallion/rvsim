//! Strong newtypes for physical and virtual addresses, ASID, IRQ id, and page numbers.

/// Number of bits to shift to convert between bytes and pages.
pub const PAGE_SHIFT: u64 = 12;

/// Mask for extracting the page offset from an address.
pub const PAGE_OFFSET_MASK: u64 = (1 << PAGE_SHIFT) - 1;

/// Mask for the bits of a VA that contribute to the 4KB VPN (Sv57 width).
///
/// Wide enough for Sv57's 45-bit VPN. Narrower modes leave the upper bits as
/// sign-extended copies of the canonical-VA top bit, so the wider mask is a
/// no-op for them.
pub const VPN_MASK: u64 = 0x1FFF_FFFF_FFFF;

/// Checks whether an unaligned access crosses a cache line boundary.
///
/// An access crosses a cache line boundary if it begins in one cache line
/// and ends in another.
///
/// # Arguments
///
/// * `addr` - The byte address of the access.
/// * `size` - The access width in bytes.
/// * `cache_line_size` - The cache line size in bytes (typically 64).
///
/// # Returns
///
/// `true` if the access spans multiple cache lines.
pub const fn crosses_cache_line(addr: u64, size: u64, cache_line_size: u64) -> bool {
    if size == 0 {
        return false;
    }
    let line_mask = cache_line_size - 1;
    (addr & line_mask) + (size - 1) >= cache_line_size
}

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VirtAddr(pub u64);

/// A physical address in the RISC-V address space.
///
/// Physical addresses represent actual hardware memory locations and are used
/// after virtual-to-physical address translation has completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PhysAddr(pub u64);

impl VirtAddr {
    /// Creates a new virtual address from a raw 64-bit value.
    #[inline(always)]
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    /// Returns the raw 64-bit address value.
    #[inline(always)]
    pub const fn val(&self) -> u64 {
        self.0
    }

    /// Returns the page offset (lower 12 bits) of the address.
    pub const fn page_offset(&self) -> u64 {
        self.0 & 0xFFF
    }
}

impl PhysAddr {
    /// Creates a new physical address from a raw 64-bit value.
    #[inline(always)]
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    /// Returns the raw 64-bit address value.
    #[inline(always)]
    pub const fn val(&self) -> u64 {
        self.0
    }
}

/// A cache-line-aligned physical address.
///
/// Wraps a [`PhysAddr`] whose low bits (log2 of the cache line size) are zero.
/// Used as the identifier for a cached line in coherence messages, MSHRs, and
/// pending-request maps. Distinct from `PhysAddr` so cache routines cannot
/// compare line-aligned addresses against byte addresses by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LineAddr(PhysAddr);

impl LineAddr {
    /// Aligns a physical address down to the cache line boundary. `line_bytes`
    /// must be a power of two.
    #[inline(always)]
    pub const fn from_phys(addr: PhysAddr, line_bytes: u64) -> Self {
        Self(PhysAddr(addr.0 & !(line_bytes - 1)))
    }

    /// Returns the underlying physical address.
    #[inline(always)]
    pub const fn phys(self) -> PhysAddr {
        self.0
    }

    /// Returns the raw 64-bit value.
    #[inline(always)]
    pub const fn val(self) -> u64 {
        self.0.0
    }
}
