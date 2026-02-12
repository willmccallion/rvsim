//! CLINT (Core Local Interruptor) Unit Tests.
//!
//! Verifies timer operation, MSIP/MTIME/MTIMECMP register read/write,
//! divider-based tick counting, and interrupt generation.

use riscv_core::soc::devices::Device;
use riscv_core::soc::devices::clint::Clint;

#[test]
fn clint_name() {
    let clint = Clint::new(0x200_0000, 10);
    assert_eq!(clint.name(), "CLINT");
}

#[test]
fn clint_address_range() {
    let clint = Clint::new(0x200_0000, 10);
    let (base, size) = clint.address_range();
    assert_eq!(base, 0x200_0000);
    assert_eq!(size, 0x10000);
}

#[test]
fn clint_initial_mtime_zero() {
    let mut clint = Clint::new(0, 1);
    assert_eq!(clint.read_u64(0xBFF8), 0);
}

#[test]
fn clint_initial_mtimecmp_max() {
    let mut clint = Clint::new(0, 1);
    assert_eq!(clint.read_u64(0x4000), u64::MAX);
}

#[test]
fn clint_tick_increments_mtime() {
    let mut clint = Clint::new(0, 1);
    // Divider = 1, so every tick increments mtime
    clint.tick();
    assert_eq!(clint.read_u64(0xBFF8), 1);
    clint.tick();
    assert_eq!(clint.read_u64(0xBFF8), 2);
}

#[test]
fn clint_tick_divider() {
    let mut clint = Clint::new(0, 10);
    // Divider = 10, mtime should only increment every 10 ticks
    for _ in 0..9 {
        clint.tick();
    }
    assert_eq!(clint.read_u64(0xBFF8), 0);
    clint.tick(); // 10th tick
    assert_eq!(clint.read_u64(0xBFF8), 1);
}

#[test]
fn clint_timer_interrupt_fires_when_mtime_ge_mtimecmp() {
    let mut clint = Clint::new(0, 1);
    clint.write_u64(0x4000, 5); // mtimecmp = 5
    for _ in 0..4 {
        assert!(!clint.tick(), "No interrupt before mtime reaches mtimecmp");
    }
    // 5th tick: mtime becomes 5, should fire
    assert!(
        clint.tick(),
        "Timer interrupt should fire when mtime >= mtimecmp"
    );
}

#[test]
fn clint_msip_write_and_read() {
    let mut clint = Clint::new(0, 1);
    clint.write_u32(0x0000, 1);
    assert_eq!(clint.read_u32(0x0000), 1);
    clint.write_u32(0x0000, 0);
    assert_eq!(clint.read_u32(0x0000), 0);
}

#[test]
fn clint_msip_only_bit_0() {
    let mut clint = Clint::new(0, 1);
    clint.write_u32(0x0000, 0xFF);
    assert_eq!(clint.read_u32(0x0000), 1, "Only bit 0 should be written");
}

#[test]
fn clint_msip_triggers_interrupt() {
    let mut clint = Clint::new(0, 1);
    clint.write_u32(0x0000, 1);
    assert!(clint.tick(), "MSIP set should trigger interrupt on tick");
}

#[test]
fn clint_write_mtime_u64() {
    let mut clint = Clint::new(0, 1);
    clint.write_u64(0xBFF8, 0x1234_5678_9ABC_DEF0);
    assert_eq!(clint.read_u64(0xBFF8), 0x1234_5678_9ABC_DEF0);
}

#[test]
fn clint_write_mtimecmp_u64() {
    let mut clint = Clint::new(0, 1);
    clint.write_u64(0x4000, 0xABCD);
    assert_eq!(clint.read_u64(0x4000), 0xABCD);
}

#[test]
fn clint_read_mtime_u32_lower() {
    let mut clint = Clint::new(0, 1);
    clint.write_u64(0xBFF8, 0x1234_5678_9ABC_DEF0);
    assert_eq!(clint.read_u32(0xBFF8), 0x9ABC_DEF0);
}

#[test]
fn clint_read_mtime_u32_upper() {
    let mut clint = Clint::new(0, 1);
    clint.write_u64(0xBFF8, 0x1234_5678_9ABC_DEF0);
    assert_eq!(clint.read_u32(0xBFF8 + 4), 0x1234_5678);
}

#[test]
fn clint_divider_zero_becomes_one() {
    // Divider of 0 should be treated as 1
    let mut clint = Clint::new(0, 0);
    clint.tick();
    assert_eq!(clint.read_u64(0xBFF8), 1);
}

#[test]
fn clint_unrecognized_offset_returns_zero() {
    let mut clint = Clint::new(0, 1);
    assert_eq!(clint.read_u64(0x1000), 0);
    assert_eq!(clint.read_u32(0x1000), 0);
}
