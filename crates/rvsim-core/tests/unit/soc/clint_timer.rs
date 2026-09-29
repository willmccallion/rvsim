//! CLINT (Core Local Interruptor) Unit Tests.
//!
//! Verifies timer operation, MSIP/MTIME/MTIMECMP register read/write,
//! divider-based tick counting, and interrupt generation.

use rvsim_core::common::{HartId, PhysAddr};
use rvsim_core::soc::devices::Device;
use rvsim_core::soc::devices::clint::Clint;

const HART0: HartId = HartId::new(0);
const HART1: HartId = HartId::new(1);

#[test]
fn clint_name() {
    let clint = Clint::new(0x200_0000, 10, 1);
    assert_eq!(clint.name(), "CLINT");
}

#[test]
fn clint_address_range() {
    let clint = Clint::new(0x200_0000, 10, 1);
    let (base, size) = clint.address_range();
    assert_eq!(base, 0x200_0000);
    assert_eq!(size, 0x10000);
}

#[test]
fn clint_initial_mtime_zero() {
    let mut clint = Clint::new(0, 1, 1);
    assert_eq!(
        crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0xBFF8), 8),
        0
    );
}

#[test]
fn clint_initial_mtimecmp_max() {
    let mut clint = Clint::new(0, 1, 1);
    assert_eq!(
        crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0x4000), 8),
        u64::MAX
    );
}

#[test]
fn clint_tick_increments_mtime() {
    let mut clint = Clint::new(0, 1, 1);
    // Divider = 1, so every tick increments mtime
    clint.tick();
    assert_eq!(
        crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0xBFF8), 8),
        1
    );
    clint.tick();
    assert_eq!(
        crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0xBFF8), 8),
        2
    );
}

#[test]
fn clint_tick_divider() {
    let mut clint = Clint::new(0, 10, 1);
    // Divider = 10, mtime should only increment every 10 ticks
    for _ in 0..9 {
        clint.tick();
    }
    assert_eq!(
        crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0xBFF8), 8),
        0
    );
    clint.tick(); // 10th tick
    assert_eq!(
        crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0xBFF8), 8),
        1
    );
}

#[test]
fn clint_timer_interrupt_fires_when_mtime_ge_mtimecmp() {
    let mut clint = Clint::new(0, 1, 1);
    crate::common::probe::write(&mut clint, rvsim_core::common::PhysAddr::new(0x4000), 5, 8); // mtimecmp = 5
    for _ in 0..4 {
        clint.tick();
        assert!(!clint.timer_pending(HART0), "No interrupt before mtime reaches mtimecmp");
    }
    // 5th tick: mtime becomes 5, should fire
    clint.tick();
    assert!(clint.timer_pending(HART0), "Timer interrupt should fire when mtime >= mtimecmp");
}

#[test]
fn clint_msip_write_and_read() {
    let mut clint = Clint::new(0, 1, 1);
    crate::common::probe::write(&mut clint, rvsim_core::common::PhysAddr::new(0x0000), 1_u64, 4);
    assert_eq!(
        (crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0x0000), 4)
            as u32),
        1
    );
    crate::common::probe::write(&mut clint, rvsim_core::common::PhysAddr::new(0x0000), 0_u64, 4);
    assert_eq!(
        (crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0x0000), 4)
            as u32),
        0
    );
}

#[test]
fn clint_msip_only_bit_0() {
    let mut clint = Clint::new(0, 1, 1);
    crate::common::probe::write(&mut clint, rvsim_core::common::PhysAddr::new(0x0000), 0xFF_u64, 4);
    assert_eq!(
        (crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0x0000), 4)
            as u32),
        1,
        "Only bit 0 should be written"
    );
}

#[test]
fn clint_msip_triggers_interrupt() {
    let mut clint = Clint::new(0, 1, 1);
    crate::common::probe::write(&mut clint, rvsim_core::common::PhysAddr::new(0x0000), 1_u64, 4);
    assert!(clint.msip_pending(HART0), "MSIP set should be reported by msip_pending()");
}

#[test]
fn clint_write_mtime_u64() {
    let mut clint = Clint::new(0, 1, 1);
    crate::common::probe::write(
        &mut clint,
        rvsim_core::common::PhysAddr::new(0xBFF8),
        0x1234_5678_9ABC_DEF0,
        8,
    );
    assert_eq!(
        crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0xBFF8), 8),
        0x1234_5678_9ABC_DEF0
    );
}

#[test]
fn clint_write_mtimecmp_u64() {
    let mut clint = Clint::new(0, 1, 1);
    crate::common::probe::write(&mut clint, rvsim_core::common::PhysAddr::new(0x4000), 0xABCD, 8);
    assert_eq!(
        crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0x4000), 8),
        0xABCD
    );
}

#[test]
fn clint_read_mtime_u32_lower() {
    let mut clint = Clint::new(0, 1, 1);
    crate::common::probe::write(
        &mut clint,
        rvsim_core::common::PhysAddr::new(0xBFF8),
        0x1234_5678_9ABC_DEF0,
        8,
    );
    assert_eq!(
        (crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0xBFF8), 4)
            as u32),
        0x9ABC_DEF0
    );
}

#[test]
fn clint_read_mtime_u32_upper() {
    let mut clint = Clint::new(0, 1, 1);
    crate::common::probe::write(
        &mut clint,
        rvsim_core::common::PhysAddr::new(0xBFF8),
        0x1234_5678_9ABC_DEF0,
        8,
    );
    assert_eq!(
        (crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0xBFF8 + 4), 4)
            as u32),
        0x1234_5678
    );
}

#[test]
fn clint_divider_zero_becomes_one() {
    // Divider of 0 should be treated as 1
    let mut clint = Clint::new(0, 0, 1);
    clint.tick();
    assert_eq!(
        crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0xBFF8), 8),
        1
    );
}

#[test]
fn clint_unrecognized_offset_returns_zero() {
    let mut clint = Clint::new(0, 1, 1);
    assert_eq!(
        crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0x1000), 8),
        0
    );
    assert_eq!(
        (crate::common::probe::read(&mut clint, rvsim_core::common::PhysAddr::new(0x1000), 4)
            as u32),
        0
    );
}

#[test]
fn clint_msip_registers_are_per_hart() {
    let mut clint = Clint::new(0, 1, 2);
    crate::common::probe::write(&mut clint, PhysAddr::new(0x0004), 1, 4);
    assert!(!clint.msip_pending(HART0));
    assert!(clint.msip_pending(HART1));
    assert_eq!(crate::common::probe::read(&mut clint, PhysAddr::new(0x0000), 4), 0);
    assert_eq!(crate::common::probe::read(&mut clint, PhysAddr::new(0x0004), 4), 1);
}

#[test]
fn clint_eight_byte_msip_access_covers_two_harts() {
    let mut clint = Clint::new(0, 1, 2);
    crate::common::probe::write(&mut clint, PhysAddr::new(0x0000), 1 | (1 << 32), 8);
    assert!(clint.msip_pending(HART0));
    assert!(clint.msip_pending(HART1));
    assert_eq!(crate::common::probe::read(&mut clint, PhysAddr::new(0x0000), 8), 1 | (1 << 32));
}

#[test]
fn clint_mtimecmp_registers_are_per_hart() {
    let mut clint = Clint::new(0, 1, 2);
    crate::common::probe::write(&mut clint, PhysAddr::new(0x4008), 3, 8);
    assert_eq!(crate::common::probe::read(&mut clint, PhysAddr::new(0x4000), 8), u64::MAX);
    assert_eq!(crate::common::probe::read(&mut clint, PhysAddr::new(0x4008), 8), 3);
    for _ in 0..3 {
        clint.tick();
    }
    assert!(!clint.timer_pending(HART0), "hart 0 keeps mtimecmp = u64::MAX");
    assert!(clint.timer_pending(HART1), "hart 1's mtimecmp of 3 has elapsed");
}

#[test]
fn clint_mtimecmp_upper_half_write_targets_the_right_hart() {
    let mut clint = Clint::new(0, 1, 2);
    crate::common::probe::write(&mut clint, PhysAddr::new(0x4008), 0, 8);
    crate::common::probe::write(&mut clint, PhysAddr::new(0x400C), 0x1234, 4);
    assert_eq!(crate::common::probe::read(&mut clint, PhysAddr::new(0x4008), 8), 0x1234 << 32);
    assert_eq!(crate::common::probe::read(&mut clint, PhysAddr::new(0x4000), 8), u64::MAX);
}

#[test]
fn clint_registers_of_absent_harts_read_zero_and_ignore_writes() {
    let mut clint = Clint::new(0, 1, 1);
    crate::common::probe::write(&mut clint, PhysAddr::new(0x0004), 1, 4);
    crate::common::probe::write(&mut clint, PhysAddr::new(0x4008), 7, 8);
    assert_eq!(crate::common::probe::read(&mut clint, PhysAddr::new(0x0004), 4), 0);
    assert_eq!(crate::common::probe::read(&mut clint, PhysAddr::new(0x4008), 8), 0);
    assert!(!clint.msip_pending(HART1));
    assert!(!clint.timer_pending(HART1));
}
