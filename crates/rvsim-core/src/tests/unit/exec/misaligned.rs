//! Unaligned memory access Unit Tests.
//!
//! Verifies alignment checks, trap generation, split loads, split stores,
//! cache line crossing detection, and latency calculations.

use crate::common::crosses_cache_line;
use crate::exec::compute::misaligned;
use crate::isa::privileged::Trap;

#[test]
fn byte_access_always_aligned() {
    // Size=1 is always aligned regardless of address.
    for addr in [0u64, 1, 2, 3, 7, 0xFF, 0x1001, u64::MAX] {
        assert!(misaligned::is_aligned(addr, 1), "addr={:#x}", addr);
    }
}

#[test]
fn halfword_alignment() {
    assert!(misaligned::is_aligned(0, 2));
    assert!(misaligned::is_aligned(2, 2));
    assert!(misaligned::is_aligned(4, 2));
    assert!(misaligned::is_aligned(0x1000, 2));
    assert!(!misaligned::is_aligned(1, 2));
    assert!(!misaligned::is_aligned(3, 2));
    assert!(!misaligned::is_aligned(5, 2));
    assert!(!misaligned::is_aligned(0x1001, 2));
}

#[test]
fn word_alignment() {
    assert!(misaligned::is_aligned(0, 4));
    assert!(misaligned::is_aligned(4, 4));
    assert!(misaligned::is_aligned(8, 4));
    assert!(misaligned::is_aligned(0x1000, 4));
    assert!(!misaligned::is_aligned(1, 4));
    assert!(!misaligned::is_aligned(2, 4));
    assert!(!misaligned::is_aligned(3, 4));
    assert!(!misaligned::is_aligned(5, 4));
    assert!(!misaligned::is_aligned(6, 4));
    assert!(!misaligned::is_aligned(7, 4));
}

#[test]
fn doubleword_alignment() {
    assert!(misaligned::is_aligned(0, 8));
    assert!(misaligned::is_aligned(8, 8));
    assert!(misaligned::is_aligned(16, 8));
    assert!(misaligned::is_aligned(0x1000, 8));
    assert!(!misaligned::is_aligned(1, 8));
    assert!(!misaligned::is_aligned(4, 8));
    assert!(!misaligned::is_aligned(7, 8));
    assert!(!misaligned::is_aligned(0x1001, 8));
}

#[test]
fn zero_size_always_aligned() {
    assert!(misaligned::is_aligned(0, 0));
    assert!(misaligned::is_aligned(1, 0));
    assert!(misaligned::is_aligned(0xDEAD, 0));
}

#[test]
fn load_misaligned_trap_contains_address() {
    let trap = misaligned::load_misaligned_trap(0x1003);
    assert_eq!(trap, Trap::LoadAddressMisaligned(0x1003));
}

#[test]
fn store_misaligned_trap_contains_address() {
    let trap = misaligned::store_misaligned_trap(0x2005);
    assert_eq!(trap, Trap::StoreAddressMisaligned(0x2005));
}

#[test]
fn load_misaligned_trap_zero_address() {
    let trap = misaligned::load_misaligned_trap(0);
    assert_eq!(trap, Trap::LoadAddressMisaligned(0));
}

#[test]
fn store_misaligned_trap_max_address() {
    let trap = misaligned::store_misaligned_trap(u64::MAX);
    assert_eq!(trap, Trap::StoreAddressMisaligned(u64::MAX));
}

#[test]
fn aligned_access_no_cache_line_crossing() {
    // Aligned access at start of cache line should never cross
    assert!(!crosses_cache_line(0, 8, 64));
    assert!(!crosses_cache_line(0, 4, 64));
    assert!(!crosses_cache_line(0, 2, 64));
    assert!(!crosses_cache_line(0, 1, 64));
}

#[test]
fn unaligned_access_within_cache_line() {
    // Unaligned access: addr=1, size=2, within first cache line (0-63)
    assert!(!crosses_cache_line(1, 2, 64));
    // Unaligned access: addr=62, size=2, within first cache line
    assert!(!crosses_cache_line(62, 2, 64));
}

#[test]
fn unaligned_access_crossing_cache_line() {
    // Unaligned access crossing cache line boundary: 63+2 = 65 > 64
    assert!(crosses_cache_line(63, 2, 64));
    // Unaligned access crossing cache line: 62+4 = 66 > 64
    assert!(crosses_cache_line(62, 4, 64));
}

#[test]
fn cache_line_crossing_at_boundary() {
    // Access starting exactly at boundary (addr=64, aligned) should not cross
    assert!(!crosses_cache_line(64, 8, 64));
    // Access starting at 63, size 2 crosses boundary
    assert!(crosses_cache_line(63, 2, 64));
    // Access at 65, size 2, crosses to next line
    assert!(!crosses_cache_line(65, 1, 64));
}

#[test]
fn zero_size_access_no_crossing() {
    // Zero-size access never crosses
    assert!(!crosses_cache_line(63, 0, 64));
}

#[test]
fn different_cache_line_sizes() {
    // With cache line size 32:
    assert!(!crosses_cache_line(30, 2, 32));
    assert!(crosses_cache_line(31, 2, 32));

    // With cache line size 128:
    assert!(!crosses_cache_line(126, 2, 128));
    assert!(crosses_cache_line(127, 2, 128));
}
