//! Unaligned memory access Unit Tests.
//!
//! Verifies alignment checks, trap generation, split loads, and split stores.

use riscv_core::common::error::Trap;
use riscv_core::core::units::lsu::unaligned;

// ══════════════════════════════════════════════════════════
// 1. Alignment checking
// ══════════════════════════════════════════════════════════

#[test]
fn byte_access_always_aligned() {
    // Size=1 is always aligned regardless of address.
    for addr in [0u64, 1, 2, 3, 7, 0xFF, 0x1001, u64::MAX] {
        assert!(unaligned::is_aligned(addr, 1), "addr={:#x}", addr);
    }
}

#[test]
fn halfword_alignment() {
    assert!(unaligned::is_aligned(0, 2));
    assert!(unaligned::is_aligned(2, 2));
    assert!(unaligned::is_aligned(4, 2));
    assert!(unaligned::is_aligned(0x1000, 2));
    assert!(!unaligned::is_aligned(1, 2));
    assert!(!unaligned::is_aligned(3, 2));
    assert!(!unaligned::is_aligned(5, 2));
    assert!(!unaligned::is_aligned(0x1001, 2));
}

#[test]
fn word_alignment() {
    assert!(unaligned::is_aligned(0, 4));
    assert!(unaligned::is_aligned(4, 4));
    assert!(unaligned::is_aligned(8, 4));
    assert!(unaligned::is_aligned(0x1000, 4));
    assert!(!unaligned::is_aligned(1, 4));
    assert!(!unaligned::is_aligned(2, 4));
    assert!(!unaligned::is_aligned(3, 4));
    assert!(!unaligned::is_aligned(5, 4));
    assert!(!unaligned::is_aligned(6, 4));
    assert!(!unaligned::is_aligned(7, 4));
}

#[test]
fn doubleword_alignment() {
    assert!(unaligned::is_aligned(0, 8));
    assert!(unaligned::is_aligned(8, 8));
    assert!(unaligned::is_aligned(16, 8));
    assert!(unaligned::is_aligned(0x1000, 8));
    assert!(!unaligned::is_aligned(1, 8));
    assert!(!unaligned::is_aligned(4, 8));
    assert!(!unaligned::is_aligned(7, 8));
    assert!(!unaligned::is_aligned(0x1001, 8));
}

#[test]
fn zero_size_always_aligned() {
    assert!(unaligned::is_aligned(0, 0));
    assert!(unaligned::is_aligned(1, 0));
    assert!(unaligned::is_aligned(0xDEAD, 0));
}

// ══════════════════════════════════════════════════════════
// 2. Trap generation
// ══════════════════════════════════════════════════════════

#[test]
fn load_misaligned_trap_contains_address() {
    let trap = unaligned::load_misaligned_trap(0x1003);
    assert_eq!(trap, Trap::LoadAddressMisaligned(0x1003));
}

#[test]
fn store_misaligned_trap_contains_address() {
    let trap = unaligned::store_misaligned_trap(0x2005);
    assert_eq!(trap, Trap::StoreAddressMisaligned(0x2005));
}

#[test]
fn load_misaligned_trap_zero_address() {
    let trap = unaligned::load_misaligned_trap(0);
    assert_eq!(trap, Trap::LoadAddressMisaligned(0));
}

#[test]
fn store_misaligned_trap_max_address() {
    let trap = unaligned::store_misaligned_trap(u64::MAX);
    assert_eq!(trap, Trap::StoreAddressMisaligned(u64::MAX));
}

// ══════════════════════════════════════════════════════════
// 3. Split load
// ══════════════════════════════════════════════════════════

#[test]
fn split_load_single_byte() {
    let mem = [0x42u8];
    let result = unaligned::split_load(0, 1, |addr| mem[addr as usize]);
    assert_eq!(result, 0x42);
}

#[test]
fn split_load_halfword_at_odd_address() {
    // Memory: [0x00, 0xAB, 0xCD, 0x00]
    let mem = [0x00u8, 0xAB, 0xCD, 0x00];
    // Load 2 bytes starting at address 1 → bytes 0xAB, 0xCD → LE = 0xCDAB
    let result = unaligned::split_load(1, 2, |addr| mem[addr as usize]);
    assert_eq!(result, 0xCDAB);
}

#[test]
fn split_load_word_at_unaligned_address() {
    // Memory at addresses 1..5: [0x11, 0x22, 0x33, 0x44]
    let mem = [0x00u8, 0x11, 0x22, 0x33, 0x44, 0x00];
    let result = unaligned::split_load(1, 4, |addr| mem[addr as usize]);
    assert_eq!(result, 0x44332211);
}

#[test]
fn split_load_doubleword() {
    let mem = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    let result = unaligned::split_load(0, 8, |addr| mem[addr as usize]);
    assert_eq!(result, 0x0807060504030201);
}

#[test]
fn split_load_aligned_word_matches_direct_read() {
    let val: u32 = 0xDEADBEEF;
    let bytes = val.to_le_bytes();
    let result = unaligned::split_load(0, 4, |addr| bytes[addr as usize]);
    assert_eq!(result, 0xDEADBEEF);
}

// ══════════════════════════════════════════════════════════
// 4. Split store
// ══════════════════════════════════════════════════════════

#[test]
fn split_store_single_byte() {
    let mut mem = [0u8; 4];
    unaligned::split_store(2, 1, 0xAB, |addr, val| mem[addr as usize] = val);
    assert_eq!(mem, [0, 0, 0xAB, 0]);
}

#[test]
fn split_store_halfword_at_odd_address() {
    let mut mem = [0u8; 4];
    // Store 0xBEEF at address 1 → LE bytes: 0xEF at addr 1, 0xBE at addr 2
    unaligned::split_store(1, 2, 0xBEEF, |addr, val| mem[addr as usize] = val);
    assert_eq!(mem, [0, 0xEF, 0xBE, 0]);
}

#[test]
fn split_store_word_at_unaligned_address() {
    let mut mem = [0u8; 8];
    unaligned::split_store(3, 4, 0xDEADBEEF, |addr, val| mem[addr as usize] = val);
    assert_eq!(mem[3], 0xEF);
    assert_eq!(mem[4], 0xBE);
    assert_eq!(mem[5], 0xAD);
    assert_eq!(mem[6], 0xDE);
    // Untouched bytes
    assert_eq!(mem[0], 0);
    assert_eq!(mem[1], 0);
    assert_eq!(mem[2], 0);
    assert_eq!(mem[7], 0);
}

#[test]
fn split_store_doubleword() {
    let mut mem = [0u8; 8];
    unaligned::split_store(0, 8, 0x0807060504030201, |addr, val| {
        mem[addr as usize] = val
    });
    assert_eq!(mem, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
}

#[test]
fn split_store_then_load_round_trip() {
    let mut mem = [0u8; 16];
    let val: u64 = 0xCAFEBABE_DEADBEEF;
    unaligned::split_store(3, 8, val, |addr, v| mem[addr as usize] = v);
    let loaded = unaligned::split_load(3, 8, |addr| mem[addr as usize]);
    assert_eq!(loaded, val);
}
