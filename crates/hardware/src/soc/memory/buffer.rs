//! DRAM Buffer Implementation.
//!
//! This module provides a safe wrapper around raw memory allocation for the system RAM.
//! It supports lazy allocation via `mmap` on Unix systems to optimize host memory usage
//! and startup time. It provides interior mutability to allow shared access between
//! the CPU (via the Memory device) and DMA-capable devices (like VirtIO).

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

impl DramBuffer {
    /// Creates a new DRAM buffer of the specified size.
    ///
    /// On Unix, uses `mmap` for lazy allocation; on other platforms, allocates a `Vec`.
    ///
    /// # Arguments
    ///
    /// * `size` - Size of the buffer in bytes.
    ///
    /// # Returns
    ///
    /// A new `DramBuffer`; panics if `mmap` fails on Unix.
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

            if ptr == libc::MAP_FAILED {
                panic!("Failed to mmap DRAM buffer of size {}", size);
            }

            Self {
                ptr: ptr as *mut u8,
                size,
                is_mmap: true,
            }
        }

        #[cfg(not(unix))]
        {
            let mut vec = vec![0u8; size];
            let ptr = vec.as_mut_ptr();
            std::mem::forget(vec);
            Self {
                ptr,
                size,
                is_mmap: false,
            }
        }
    }

    /// Returns the size of the buffer in bytes.
    pub fn len(&self) -> usize {
        self.size
    }

    /// Returns a raw pointer to the buffer.
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    /// Returns a mutable raw pointer to the buffer.
    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// Reads a single byte safely.
    pub fn read_u8(&self, offset: usize) -> u8 {
        assert!(offset < self.size, "DRAM read out of bounds");
        unsafe { *self.ptr.add(offset) }
    }

    /// Writes a single byte safely.
    pub fn write_u8(&self, offset: usize, val: u8) {
        assert!(offset < self.size, "DRAM write out of bounds");
        unsafe {
            *self.ptr.add(offset) = val;
        }
    }

    /// Reads a slice of memory safely.
    pub fn read_slice(&self, offset: usize, len: usize) -> &[u8] {
        assert!(offset + len <= self.size, "DRAM read out of bounds");
        unsafe { slice::from_raw_parts(self.ptr.add(offset), len) }
    }

    /// Writes a slice of memory safely.
    pub fn write_slice(&self, offset: usize, data: &[u8]) {
        assert!(offset + data.len() <= self.size, "DRAM write out of bounds");
        unsafe {
            let dest = self.ptr.add(offset);
            std::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());
        }
    }
}

impl Drop for DramBuffer {
    /// Deallocates the DRAM buffer.
    ///
    /// On Unix systems, unmaps the mmap'd memory. On other systems,
    /// reconstructs the Vec to trigger its destructor.
    fn drop(&mut self) {
        if self.is_mmap {
            #[cfg(unix)]
            unsafe {
                libc::munmap(self.ptr as *mut _, self.size);
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
    /// Output type for indexing operations (u8).
    type Output = u8;

    /// Indexes into the buffer to read a byte.
    fn index(&self, index: usize) -> &Self::Output {
        unsafe { &*self.ptr.add(index) }
    }
}

impl IndexMut<usize> for DramBuffer {
    /// Indexes into the buffer to write a byte.
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        unsafe { &mut *self.ptr.add(index) }
    }
}
