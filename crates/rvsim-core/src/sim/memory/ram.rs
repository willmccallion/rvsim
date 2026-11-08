//! Physical RAM: a zeroed byte image mapped at a base address.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::fmt;
use std::io;
use std::ptr::NonNull;

use crate::common::PhysAddr;

/// `size` zeroed bytes of RAM at `base`.
///
/// The image is uniquely owned, so every read borrows it and every write
/// borrows it mutably; nothing else can alias its bytes.
#[derive(Debug)]
pub struct Ram {
    bytes: ZeroedBytes,
    base: PhysAddr,
}

impl Ram {
    /// `size` zeroed bytes at `base`.
    ///
    /// # Errors
    ///
    /// Returns the host's error when it cannot map `size` bytes, or one
    /// saying so when `size` is more than the host can address.
    pub fn new(base: PhysAddr, size: u64) -> io::Result<Self> {
        let len = usize::try_from(size)
            .map_err(|_| io::Error::other("RAM larger than the host's address space"))?;
        Ok(Self { bytes: ZeroedBytes::new(len)?, base })
    }

    /// The first physical address of the image.
    #[must_use]
    pub const fn base(&self) -> PhysAddr {
        self.base
    }

    /// The image's size in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.bytes.len as u64
    }

    /// True when `[addr, addr + len)` lies inside the image.
    #[must_use]
    pub const fn contains(&self, addr: PhysAddr, len: u64) -> bool {
        let addr = addr.val();
        let base = self.base.val();
        addr >= base && addr.saturating_add(len) <= base.saturating_add(self.size())
    }

    /// Every byte of the image, in address order.
    #[must_use]
    pub const fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Every byte of the image, to overwrite.
    pub const fn bytes_mut(&mut self) -> &mut [u8] {
        self.bytes.as_mut_slice()
    }

    /// The `len` bytes at `addr`; `None` when any of them is outside the
    /// image.
    #[must_use]
    pub fn get(&self, addr: PhysAddr, len: usize) -> Option<&[u8]> {
        let offset = self.offset(addr, len)?;
        self.bytes.as_slice().get(offset..offset + len)
    }

    /// The `len` bytes at `addr`, to overwrite; `None` when any of them is
    /// outside the image.
    pub fn get_mut(&mut self, addr: PhysAddr, len: usize) -> Option<&mut [u8]> {
        let offset = self.offset(addr, len)?;
        self.bytes.as_mut_slice().get_mut(offset..offset + len)
    }

    fn offset(&self, addr: PhysAddr, len: usize) -> Option<usize> {
        self.contains(addr, len as u64).then(|| (addr.val() - self.base.val()) as usize)
    }
}

/// An owned zero-filled allocation from the global allocator. A large one
/// comes from the system's zeroed allocation, which the kernel backs lazily,
/// so a large RAM costs memory only for the pages the guest touches.
struct ZeroedBytes {
    ptr: NonNull<u8>,
    len: usize,
}

impl fmt::Debug for ZeroedBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZeroedBytes").field("len", &self.len).finish_non_exhaustive()
    }
}

// SAFETY: the allocation is owned by exactly one `ZeroedBytes`, every
// access goes through `&self` / `&mut self` slices, and it may be freed
// from any thread.
unsafe impl Send for ZeroedBytes {}

// SAFETY: shared access only yields `&[u8]`; mutation needs `&mut self`.
unsafe impl Sync for ZeroedBytes {}

impl ZeroedBytes {
    fn new(len: usize) -> io::Result<Self> {
        if len == 0 {
            return Ok(Self { ptr: NonNull::dangling(), len });
        }
        let layout = Self::layout(len)?;
        // SAFETY: `layout` has a non-zero size, checked above.
        let raw = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(raw)
            .ok_or_else(|| io::Error::new(io::ErrorKind::OutOfMemory, "cannot allocate the RAM"))?;
        Ok(Self { ptr, len })
    }

    /// Byte-aligned, which keeps a large allocation on the lazily zeroed path.
    fn layout(len: usize) -> io::Result<Layout> {
        Layout::from_size_align(len, 1).map_err(io::Error::other)
    }

    const fn as_slice(&self) -> &[u8] {
        // SAFETY: `ptr` is a live readable allocation of `len` bytes (or
        // dangling with `len == 0`), and `&self` keeps it from being freed
        // or written while the slice lives.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    const fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as `as_slice`, and `&mut self` makes this the only access.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for ZeroedBytes {
    fn drop(&mut self) {
        if self.len == 0 {
            return;
        }
        let Ok(layout) = Self::layout(self.len) else { return };
        // SAFETY: `ptr` came from `alloc_zeroed` with this same layout, and
        // no slice of it outlives `self`.
        unsafe { dealloc(self.ptr.as_ptr(), layout) };
    }
}
