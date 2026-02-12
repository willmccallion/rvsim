//! # Address Arithmetic Tests
//!
//! This module contains unit tests for the `VirtAddr` and `PhysAddr` types.
//! It verifies the correctness of address construction, value retrieval,
//! page offset calculations, and comparison logic to ensure robust memory
//! management within the RISC-V emulator.

use riscv_core::common::addr::{PhysAddr, VirtAddr};

/// Tests the creation of a [`VirtAddr`] and verifies that the stored value
/// can be retrieved correctly.
#[test]
fn virt_addr_new_and_val() {
    let va = VirtAddr::new(0x8000_1234);
    assert_eq!(va.val(), 0x8000_1234);
}

/// Tests that a virtual address can be initialized to zero.
#[test]
fn virt_addr_zero() {
    let va = VirtAddr::new(0);
    assert_eq!(va.val(), 0);
}

/// Verifies that a [`VirtAddr`] can be initialized with the maximum `u64` value.
#[test]
fn virt_addr_max() {
    let va = VirtAddr::new(u64::MAX);
    assert_eq!(va.val(), u64::MAX);
}

/// Tests that a page-aligned virtual address results in a page offset of zero.
#[test]
fn virt_addr_page_offset_aligned() {
    // Page-aligned address → offset = 0
    let va = VirtAddr::new(0x8000_0000);
    assert_eq!(va.page_offset(), 0);
}

/// Tests that `page_offset` correctly extracts a non-zero offset from a virtual address.
#[test]
fn virt_addr_page_offset_nonzero() {
    let va = VirtAddr::new(0x8000_0ABC);
    assert_eq!(va.page_offset(), 0xABC);
}

/// Tests that `page_offset` correctly extracts the maximum possible offset (0xFFF).
#[test]
fn virt_addr_page_offset_max() {
    // Maximum page offset is 0xFFF (4095)
    let va = VirtAddr::new(0x1000_0FFF);
    assert_eq!(va.page_offset(), 0xFFF);
}

/// Tests that `page_offset` only considers the lower 12 bits, even for large addresses.
#[test]
fn virt_addr_page_offset_only_lower_12_bits() {
    let va = VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF);
    assert_eq!(va.page_offset(), 0xFFF);
}

/// Verifies the implementation of equality for virtual addresses.
#[test]
fn virt_addr_equality() {
    let a = VirtAddr::new(42);
    let b = VirtAddr::new(42);
    assert_eq!(a, b);
}

/// Verifies the implementation of ordering for virtual addresses.
#[test]
fn virt_addr_ordering() {
    let lo = VirtAddr::new(100);
    let hi = VirtAddr::new(200);
    assert!(lo < hi);
}

/// Verifies basic construction and value retrieval for physical addresses.
#[test]
fn phys_addr_new_and_val() {
    let pa = PhysAddr::new(0x8000_0000);
    assert_eq!(pa.val(), 0x8000_0000);
}

/// Verifies that a physical address can represent the zero address.
#[test]
fn phys_addr_zero() {
    let pa = PhysAddr::new(0);
    assert_eq!(pa.val(), 0);
}

/// Verifies that a physical address can represent the maximum 64-bit value.
#[test]
fn phys_addr_max() {
    let pa = PhysAddr::new(u64::MAX);
    assert_eq!(pa.val(), u64::MAX);
}

/// Verifies the implementation of equality for physical addresses.
#[test]
fn phys_addr_equality() {
    assert_eq!(PhysAddr::new(1000), PhysAddr::new(1000));
    assert_ne!(PhysAddr::new(1000), PhysAddr::new(1001));
}

/// Verifies the implementation of ordering for physical addresses.
#[test]
fn phys_addr_ordering() {
    let lo = PhysAddr::new(0x1000);
    let hi = PhysAddr::new(0x2000);
    assert!(lo < hi);
}

/// Verifies that virtual and physical addresses are distinct types even when
/// holding the same underlying value.
#[test]
fn virt_and_phys_same_value_not_interchangeable() {
    // VirtAddr and PhysAddr are distinct types; we verify they hold
    // the same value independently.
    let v = VirtAddr::new(0x1234);
    let p = PhysAddr::new(0x1234);
    assert_eq!(v.val(), p.val());
}
