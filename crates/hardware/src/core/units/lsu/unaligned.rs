//! Unaligned memory access handling.
//!
//! This module implements support for unaligned loads and stores,
//! splitting them into multiple aligned byte reads/writes when the
//! target does not support hardware unaligned access, and providing
//! alignment checking utilities.

use crate::common::error::Trap;

/// Checks whether a memory access at `addr` with `size` bytes is naturally aligned.
///
/// # Arguments
///
/// * `addr` - The byte address of the access.
/// * `size` - The access width in bytes (1, 2, 4, or 8).
///
/// # Returns
///
/// `true` if the access is naturally aligned.
pub fn is_aligned(addr: u64, size: u64) -> bool {
    if size == 0 || size == 1 {
        return true;
    }
    (addr & (size - 1)) == 0
}

/// Returns the appropriate misaligned trap for a load at `addr`.
///
/// # Arguments
///
/// * `addr` - The misaligned address.
///
/// # Returns
///
/// `Trap::LoadAddressMisaligned` with the faulting address.
pub fn load_misaligned_trap(addr: u64) -> Trap {
    Trap::LoadAddressMisaligned(addr)
}

/// Returns the appropriate misaligned trap for a store at `addr`.
///
/// # Arguments
///
/// * `addr` - The misaligned address.
///
/// # Returns
///
/// `Trap::StoreAddressMisaligned` with the faulting address.
pub fn store_misaligned_trap(addr: u64) -> Trap {
    Trap::StoreAddressMisaligned(addr)
}

/// Splits an unaligned load into multiple byte reads and reassembles
/// the result in little-endian order.
///
/// # Arguments
///
/// * `addr` - The byte address of the access.
/// * `size` - The number of bytes to read (1, 2, 4, or 8).
/// * `read_byte` - A closure that reads a single byte from the given address.
///
/// # Returns
///
/// The reassembled value in little-endian byte order.
pub fn split_load<F>(addr: u64, size: u64, mut read_byte: F) -> u64
where
    F: FnMut(u64) -> u8,
{
    let mut result: u64 = 0;
    for i in 0..size {
        let byte = read_byte(addr + i) as u64;
        result |= byte << (i * 8);
    }
    result
}

/// Splits an unaligned store into multiple byte writes.
///
/// # Arguments
///
/// * `addr` - The byte address of the access.
/// * `size` - The number of bytes to write (1, 2, 4, or 8).
/// * `val` - The value to store (little-endian).
/// * `write_byte` - A closure that writes a single byte to the given address.
pub fn split_store<F>(addr: u64, size: u64, val: u64, mut write_byte: F)
where
    F: FnMut(u64, u8),
{
    for i in 0..size {
        let byte = ((val >> (i * 8)) & 0xFF) as u8;
        write_byte(addr + i, byte);
    }
}
