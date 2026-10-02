//! Physical RAM: a zeroed byte image mapped at a base address.

use std::fmt;
use std::ptr::NonNull;

use crate::common::PhysAddr;

/// `size` zeroed bytes of RAM at `base`.
///
/// The image is uniquely owned, so every read borrows it and every write
/// borrows it mutably; nothing else can alias its bytes.
#[derive(Debug)]
pub struct Ram {
    bytes: ZeroedBytes,
    base: u64,
}

impl Ram {
    /// `size` zeroed bytes at `base`.
    ///
    /// # Panics
    ///
    /// Panics if the host cannot map `size` bytes.
    #[must_use]
    pub fn new(base: u64, size: usize) -> Self {
        Self { bytes: ZeroedBytes::new(size), base }
    }

    /// The first physical address of the image.
    #[must_use]
    pub const fn base(&self) -> u64 {
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
        addr >= self.base && addr.saturating_add(len) <= self.base.saturating_add(self.size())
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
        self.contains(addr, len as u64).then(|| (addr.val() - self.base) as usize)
    }
}

/// An owned zero-filled allocation. On Unix it is an anonymous mapping the
/// kernel backs lazily, so a large RAM costs memory only for the pages the
/// guest touches; elsewhere it is a heap allocation.
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
    fn new(len: usize) -> Self {
        if len == 0 {
            return Self { ptr: NonNull::dangling(), len };
        }
        Self { ptr: Self::allocate(len), len }
    }

    #[cfg(unix)]
    fn allocate(len: usize) -> NonNull<u8> {
        // SAFETY: an anonymous private mapping has no file or address
        // requirements; the arguments are the documented flags for one.
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        assert!(raw != libc::MAP_FAILED, "cannot map {len} bytes of RAM");
        NonNull::new(raw.cast::<u8>()).unwrap_or(NonNull::dangling())
    }

    #[cfg(not(unix))]
    fn allocate(len: usize) -> NonNull<u8> {
        let raw = Box::into_raw(vec![0u8; len].into_boxed_slice());
        NonNull::new(raw.cast::<u8>()).unwrap_or(NonNull::dangling())
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
        #[cfg(unix)]
        // SAFETY: `ptr` and `len` are what `mmap` returned, and no slice of
        // the mapping outlives `self`.
        let _ = unsafe { libc::munmap(self.ptr.as_ptr().cast(), self.len) };
        #[cfg(not(unix))]
        // SAFETY: `ptr` and `len` are what `Box::into_raw` gave for a boxed
        // slice of `len` bytes, and no slice of it outlives `self`.
        drop(unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(self.ptr.as_ptr(), self.len))
        });
    }
}
