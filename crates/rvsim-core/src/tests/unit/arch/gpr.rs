//! # General-Purpose Register Tests
//!
//! Tests for the RISC-V general-purpose register file implementation.

use crate::arch::regs::gpr::Gpr;
use crate::isa::reg::RegIdx;

#[test]
fn a_new_register_file_reads_zero_everywhere() {
    let gpr = Gpr::new();
    for i in 0u8..32 {
        assert_eq!(gpr.read(RegIdx::new(i)), 0);
    }
}

#[test]
fn x0_reads_zero_after_a_write() {
    let mut gpr = Gpr::new();
    gpr.write(RegIdx::new(0), 0xDEAD_BEEF);
    assert_eq!(gpr.read(RegIdx::new(0)), 0);
}

#[test]
fn x1_reads_back_a_written_value() {
    let mut gpr = Gpr::new();
    let value = 0x1234_5678;
    gpr.write(RegIdx::new(1), value);
    assert_eq!(gpr.read(RegIdx::new(1)), value);
}

#[test]
fn x31_reads_back_a_written_value() {
    let mut gpr = Gpr::new();
    let value = 0x9999_AAAA;
    gpr.write(RegIdx::new(31), value);
    assert_eq!(gpr.read(RegIdx::new(31)), value);
}

#[test]
fn every_register_but_x0_reads_back_its_write() {
    let mut gpr = Gpr::new();
    for i in 1u8..32 {
        let value = u64::from(i) << 32 | u64::from(i);
        gpr.write(RegIdx::new(i), value);
        assert_eq!(gpr.read(RegIdx::new(i)), value);
    }
}

#[test]
fn x0_reads_zero_after_writes_of_several_values() {
    let mut gpr = Gpr::new();
    for value in [1u64, 0xFFFF_FFFF, 0x8000_0000] {
        gpr.write(RegIdx::new(0), value);
        assert_eq!(gpr.read(RegIdx::new(0)), 0);
    }
}

#[test]
fn a_later_write_replaces_the_earlier_value() {
    let mut gpr = Gpr::new();
    gpr.write(RegIdx::new(5), 100);
    assert_eq!(gpr.read(RegIdx::new(5)), 100);
    gpr.write(RegIdx::new(5), 200);
    assert_eq!(gpr.read(RegIdx::new(5)), 200);
    gpr.write(RegIdx::new(5), 300);
    assert_eq!(gpr.read(RegIdx::new(5)), 300);
}

#[test]
fn writes_to_different_registers_do_not_interfere() {
    let mut gpr = Gpr::new();
    gpr.write(RegIdx::new(1), 111);
    gpr.write(RegIdx::new(2), 222);
    gpr.write(RegIdx::new(3), 333);

    assert_eq!(gpr.read(RegIdx::new(1)), 111);
    assert_eq!(gpr.read(RegIdx::new(2)), 222);
    assert_eq!(gpr.read(RegIdx::new(3)), 333);
}

#[test]
fn a_register_holds_u64_max() {
    let mut gpr = Gpr::new();
    let large_value = u64::MAX;
    gpr.write(RegIdx::new(10), large_value);
    assert_eq!(gpr.read(RegIdx::new(10)), large_value);
}

#[test]
fn x0_stays_zero_after_writes_to_every_other_register() {
    let mut gpr = Gpr::new();
    // Write to other registers
    for i in 1u8..32 {
        gpr.write(RegIdx::new(i), 0x1111_1111);
    }
    // x0 should still be zero
    assert_eq!(gpr.read(RegIdx::new(0)), 0);
}

#[test]
fn display_prints_registers_as_zero_padded_hex() {
    let mut gpr = Gpr::new();
    gpr.write(RegIdx::new(1), 0x1234_5678);
    gpr.write(RegIdx::new(31), 0xFFFF_FFFF);
    let text = gpr.to_string();
    assert!(text.contains("x1 =0x0000000012345678"), "{text}");
    assert!(text.contains("x31=0x00000000ffffffff"), "{text}");
}

#[test]
fn registers_read_back_each_round_of_rewrites() {
    let mut gpr = Gpr::new();
    for iteration in 0..3 {
        for i in 1u8..32 {
            let value = u64::from(i) + (iteration as u64) * 100;
            gpr.write(RegIdx::new(i), value);
        }
        for i in 1u8..32 {
            let expected = u64::from(i) + (iteration as u64) * 100;
            assert_eq!(gpr.read(RegIdx::new(i)), expected);
        }
    }
}
