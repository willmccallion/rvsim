//! # General-Purpose Register Tests
//!
//! Tests for the RISC-V general-purpose register file implementation.

use rvsim_core::core::arch::gpr::Gpr;

#[test]
fn test_gpr_new_initializes_to_zero() {
    let gpr = Gpr::new();
    for i in 0..32 {
        assert_eq!(gpr.read(i), 0);
    }
}

#[test]
fn test_gpr_read_write_x0_always_zero() {
    let mut gpr = Gpr::new();
    gpr.write(0, 0xDEAD_BEEF);
    assert_eq!(gpr.read(0), 0);
}

#[test]
fn test_gpr_read_write_x1() {
    let mut gpr = Gpr::new();
    let value = 0x1234_5678;
    gpr.write(1, value);
    assert_eq!(gpr.read(1), value);
}

#[test]
fn test_gpr_read_write_x31() {
    let mut gpr = Gpr::new();
    let value = 0x9999_AAAA;
    gpr.write(31, value);
    assert_eq!(gpr.read(31), value);
}

#[test]
fn test_gpr_write_all_registers() {
    let mut gpr = Gpr::new();
    for i in 1..32 {
        let value = (i as u64) << 32 | (i as u64);
        gpr.write(i, value);
        assert_eq!(gpr.read(i), value);
    }
}

#[test]
fn test_gpr_x0_ignores_writes() {
    let mut gpr = Gpr::new();
    for value in [1u64, 0xFFFF_FFFF, 0x8000_0000] {
        gpr.write(0, value);
        assert_eq!(gpr.read(0), 0);
    }
}

#[test]
fn test_gpr_multiple_writes_to_same_register() {
    let mut gpr = Gpr::new();
    gpr.write(5, 100);
    assert_eq!(gpr.read(5), 100);
    gpr.write(5, 200);
    assert_eq!(gpr.read(5), 200);
    gpr.write(5, 300);
    assert_eq!(gpr.read(5), 300);
}

#[test]
fn test_gpr_register_independence() {
    let mut gpr = Gpr::new();
    gpr.write(1, 111);
    gpr.write(2, 222);
    gpr.write(3, 333);

    assert_eq!(gpr.read(1), 111);
    assert_eq!(gpr.read(2), 222);
    assert_eq!(gpr.read(3), 333);
}

#[test]
fn test_gpr_large_values() {
    let mut gpr = Gpr::new();
    let large_value = u64::MAX;
    gpr.write(10, large_value);
    assert_eq!(gpr.read(10), large_value);
}

#[test]
fn test_gpr_zero_after_writes() {
    let mut gpr = Gpr::new();
    // Write to other registers
    for i in 1..32 {
        gpr.write(i, 0x1111_1111);
    }
    // x0 should still be zero
    assert_eq!(gpr.read(0), 0);
}

#[test]
fn test_gpr_dump_does_not_panic() {
    let mut gpr = Gpr::new();
    gpr.write(1, 0x1234_5678);
    gpr.write(31, 0xFFFF_FFFF);
    gpr.dump(); // Should not panic
}

#[test]
fn test_gpr_alternating_write_read() {
    let mut gpr = Gpr::new();
    for iteration in 0..3 {
        for i in 1..32 {
            let value = (i as u64) + (iteration as u64) * 100;
            gpr.write(i, value);
        }
        for i in 1..32 {
            let expected = (i as u64) + (iteration as u64) * 100;
            assert_eq!(gpr.read(i), expected);
        }
    }
}
