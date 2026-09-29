//! DRAM Buffer Implementation.
//!
//! This module provides a safe wrapper around raw memory allocation for the system RAM.
//! It supports lazy allocation via `mmap` on Unix systems to optimize host memory usage
//! and startup time. It provides interior mutability to allow shared access between
//! the CPU (via the Memory device) and DMA-capable devices (like virtio).

use std::ops::{Index, IndexMut};
use std::slice;

/// A simplified wrapper around a raw memory buffer.
///
/// On Unix systems, this uses `mmap` to allocate anonymous memory, which allows
/// for lazy allocation (pages are only allocated by the OS when accessed).
/// This significantly improves startup time and memory pressure for large RAM sizes.
pub struct DramBuffer {
    ptr: *mut u8,
    size: usize,
    is_mmap: bool,
}

unsafe impl Send for DramBuffer {}
unsafe impl Sync for DramBuffer {}

impl std::fmt::Debug for DramBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DramBuffer")
            .field("size", &self.size)
            .field("is_mmap", &self.is_mmap)
            .finish_non_exhaustive()
    }
}

impl DramBuffer {
    /// Creates a new DRAM buffer of `size` bytes.
    /// On Unix, uses `mmap` for lazy allocation; otherwise allocates a `Vec`.
    ///
    /// # Panics
    ///
    /// Panics if `mmap` fails on Unix.
    pub fn new(size: usize) -> Self {
        #[cfg(unix)]
        {
            use std::ptr;
            let ptr = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    size,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };

            assert!(ptr != libc::MAP_FAILED, "Failed to mmap DRAM buffer of size {size}");

            Self { ptr: ptr as *mut u8, size, is_mmap: true }
        }

        #[cfg(not(unix))]
        {
            let mut vec = vec![0u8; size];
            let ptr = vec.as_mut_ptr();
            std::mem::forget(vec);
            Self { ptr, size, is_mmap: false }
        }
    }

    /// Returns the size of the buffer in bytes.
    pub const fn len(&self) -> usize {
        self.size
    }

    /// Returns true if the buffer has zero size.
    pub const fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Returns a raw pointer to the buffer.
    pub const fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    /// Returns a mutable raw pointer to the buffer.
    pub const fn as_mut_ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// Reads a single byte.
    ///
    /// # Panics
    ///
    /// Panics if `offset` is out of bounds.
    pub fn read_u8(&self, offset: usize) -> u8 {
        assert!(offset < self.size, "DRAM read out of bounds");
        unsafe { *self.ptr.add(offset) }
    }

    /// Writes a single byte.
    ///
    /// # Panics
    ///
    /// Panics if `offset` is out of bounds.
    pub fn write_u8(&self, offset: usize, val: u8) {
        assert!(offset < self.size, "DRAM write out of bounds");
        unsafe {
            *self.ptr.add(offset) = val;
        }
    }

    /// Reads a slice.
    ///
    /// # Panics
    ///
    /// Panics if `offset + len` is out of bounds.
    pub fn read_slice(&self, offset: usize, len: usize) -> &[u8] {
        assert!(offset + len <= self.size, "DRAM read out of bounds");
        unsafe { slice::from_raw_parts(self.ptr.add(offset), len) }
    }

    /// Writes a slice.
    ///
    /// # Panics
    ///
    /// Panics if `offset + data.len()` is out of bounds.
    pub fn write_slice(&self, offset: usize, data: &[u8]) {
        assert!(offset + data.len() <= self.size, "DRAM write out of bounds");
        unsafe {
            let dest = self.ptr.add(offset);
            std::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());
        }
    }
}

impl Drop for DramBuffer {
    fn drop(&mut self) {
        if self.is_mmap {
            #[cfg(unix)]
            unsafe {
                let _ = libc::munmap(self.ptr as *mut _, self.size);
            }
        } else {
            #[cfg(not(unix))]
            unsafe {
                let _ = Vec::from_raw_parts(self.ptr, self.size, self.size);
            }
        }
    }
}

impl Index<usize> for DramBuffer {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        unsafe { &*self.ptr.add(index) }
    }
}

impl IndexMut<usize> for DramBuffer {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        unsafe { &mut *self.ptr.add(index) }
    }
}
