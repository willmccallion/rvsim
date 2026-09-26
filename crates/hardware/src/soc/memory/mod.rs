//! Physical System Memory (DRAM): backing buffer, mapping device, and latency controller.

/// Physical-address ↔ DRAM-coordinate mapping.
pub mod address;

/// DRAM buffer implementation (e.g., mmap or `Vec`) for raw byte storage.
pub mod buffer;

/// Memory controller implementations for access latency modeling.
pub mod controller;

/// DDR5 controller with per-bank command state machines and JEDEC timing.
pub mod ddr5;

/// Direct view of a contiguous physical RAM region. Lets the pipeline
/// bypass bus device dispatch on the hot load/store path.
///
/// `Copy`-able so `Bus::ram_region()` can return it by value without
/// extending any borrows on the bus itself.
#[derive(Clone, Copy, Debug)]
pub struct RamRegion {
    ptr: *mut u8,
    base: u64,
    size: u64,
}

// Pipeline accesses RamRegion across thread boundaries (sweep workers run
// independent simulations); the underlying buffer is owned by an Arc on
// the Memory device, so the pointer is valid for the simulator's lifetime.
unsafe impl Send for RamRegion {}
unsafe impl Sync for RamRegion {}

impl RamRegion {
    /// Constructs a region from the Memory device's raw pointer and address range.
    pub const fn new(ptr: *mut u8, base: u64, size: u64) -> Self {
        Self { ptr, base, size }
    }

    /// Base physical address of the region.
    #[inline]
    pub const fn base(&self) -> u64 {
        self.base
    }

    /// Size of the region in bytes.
    #[inline]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Raw pointer to the start of the region.
    #[inline]
    pub const fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// True when `[addr, addr+len)` is fully inside this region.
    #[inline]
    pub const fn contains(&self, addr: u64, len: u64) -> bool {
        addr >= self.base && addr.saturating_add(len) <= self.base.saturating_add(self.size)
    }

    /// Returns the raw byte pointer for the given physical address.
    ///
    /// # Safety
    ///
    /// Caller must verify [`Self::contains`] for `addr` and the access
    /// width before dereferencing.
    #[inline]
    pub const unsafe fn ptr(&self, addr: u64) -> *mut u8 {
        // SAFETY: caller guarantees the offset is in-bounds via `contains`.
        unsafe { self.ptr.add((addr - self.base) as usize) }
    }
}
