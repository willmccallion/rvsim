//! Unaligned memory access handling.
//!
//! This module implements hardware support for unaligned loads and stores,
//! splitting them into multiple aligned byte reads/writes when the access
//! is not naturally aligned. It also provides:
//! - Alignment checking utilities
//! - Cache line crossing detection
//! - Latency calculation for unaligned accesses
//! - Support for byte-granular split access

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
pub const fn is_aligned(addr: u64, size: u64) -> bool {
    if size == 0 || size == 1 {
        return true;
    }
    (addr & (size - 1)) == 0
}

/// Checks whether an unaligned access crosses a cache line boundary.
///
/// An access crosses a cache line boundary if it begins in one cache line
/// and ends in another. This is important for modeling latency penalties.
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

/// Calculates the latency penalty (in cycles) for an unaligned access.
///
/// Unaligned accesses that stay within a cache line incur a small penalty (1-2 cycles).
/// Accesses that cross a cache line boundary incur a larger penalty (2-3+ cycles) due to
/// the possibility of two cache misses instead of one.
///
/// # Arguments
///
/// * `addr` - The byte address of the access.
/// * `size` - The access width in bytes.
/// * `cache_line_size` - The cache line size in bytes.
///
/// # Returns
///
/// The additional latency penalty in cycles for this unaligned access.
/// Returns 0 for aligned accesses.
pub const fn calculate_unaligned_latency(addr: u64, size: u64, cache_line_size: u64) -> u64 {
    // Aligned accesses have no penalty
    if is_aligned(addr, size) {
        return 0;
    }

    // Unaligned access within a cache line: 1 cycle penalty
    if !crosses_cache_line(addr, size, cache_line_size) {
        return 1;
    }

    // Unaligned access crossing cache line: 2 cycles penalty (potential two cache accesses)
    2
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
pub const fn load_misaligned_trap(addr: u64) -> Trap {
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
pub const fn store_misaligned_trap(addr: u64) -> Trap {
    Trap::StoreAddressMisaligned(addr)
}

/// Returns the size in bytes for a memory operation width.
///
/// # Arguments
///
/// * `width` - The `MemWidth` enum value
///
/// # Returns
///
/// The size in bytes (0 for Nop, 1/2/4/8 for actual operations)
pub const fn width_to_bytes(width: crate::core::pipeline::signals::MemWidth) -> u64 {
    use crate::core::pipeline::signals::MemWidth;
    match width {
        MemWidth::Nop => 0,
        MemWidth::Byte => 1,
        MemWidth::Half => 2,
        MemWidth::Word => 4,
        MemWidth::Double => 8,
    }
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
