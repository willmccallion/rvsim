//! # SysCon Device Tests
//!
//! This module contains unit tests for the System Controller (SysCon) device,
//! which provides power management and reset functionality through memory-mapped
//! registers.

use crate::common::PhysAddr;
use crate::soc::devices::Device;
use crate::soc::devices::syscon::SysCon;
use crate::soc::exit::ExitSignal;

/// Helper function to create a test SysCon device.
fn create_test_syscon() -> (SysCon, ExitSignal) {
    let base_addr = 0x100000;
    let exit_signal = ExitSignal::new();
    let syscon = SysCon::new(PhysAddr::new(base_addr), exit_signal.clone());
    (syscon, exit_signal)
}

#[test]
fn test_syscon_name() {
    let (syscon, _) = create_test_syscon();
    assert_eq!(syscon.name(), "SysCon");
}

#[test]
fn test_syscon_address_range() {
    let base_addr = 0x100000;
    let exit_signal = ExitSignal::new();
    let syscon = SysCon::new(PhysAddr::new(base_addr), exit_signal);

    let region = syscon.region();
    let (base, size) = (region.base.val(), region.size);
    assert_eq!(base, base_addr);
    assert_eq!(size, 0x1000);
}

#[test]
fn test_syscon_read_u8_returns_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(
        (crate::tests::support::probe::read(&mut syscon, crate::common::PhysAddr::new(0x100000), 1)
            as u8),
        0
    );
    assert_eq!(
        (crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000 + 0x100),
            1
        ) as u8),
        0
    );
}

#[test]
fn test_syscon_read_u16_returns_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(
        (crate::tests::support::probe::read(&mut syscon, crate::common::PhysAddr::new(0x100000), 2)
            as u16),
        0
    );
    assert_eq!(
        (crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000 + 0x100),
            2
        ) as u16),
        0
    );
}

#[test]
fn test_syscon_read_u32_returns_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(
        (crate::tests::support::probe::read(&mut syscon, crate::common::PhysAddr::new(0x100000), 4)
            as u32),
        0
    );
    assert_eq!(
        (crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000 + 0x100),
            4
        ) as u32),
        0
    );
}

#[test]
fn test_syscon_read_u64_returns_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(
        crate::tests::support::probe::read(&mut syscon, crate::common::PhysAddr::new(0x100000), 8),
        0
    );
    assert_eq!(
        crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000 + 0x100),
            8
        ),
        0
    );
}

#[test]
fn test_syscon_write_u8_ignored() {
    let (mut syscon, exit_signal) = create_test_syscon();

    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x55_u64,
        1,
    );
    // Exit signal should remain unchanged
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn test_syscon_write_u16_ignored() {
    let (mut syscon, exit_signal) = create_test_syscon();

    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x5555_u64,
        2,
    );
    // Exit signal should remain unchanged
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn test_syscon_poweroff_signal() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write poweroff magic value (0x5555) to offset 0
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x5555_u64,
        4,
    );

    // Exit signal should be set to 0
    assert_eq!(exit_signal.peek(), Some(0));
}

#[test]
fn test_syscon_reset_signal() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write reset magic value (0x7777) to offset 0
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x7777_u64,
        4,
    );

    // Exit signal should be set to 0 (simulated as exit)
    assert_eq!(exit_signal.peek(), Some(0));
}

#[test]
fn test_syscon_failure_signal() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write failure magic value (0x3333) to offset 0
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x3333_u64,
        4,
    );

    // Exit signal should be set to 1 (failure)
    assert_eq!(exit_signal.peek(), Some(1));
}

#[test]
fn test_syscon_write_non_magic_value_ignored() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write non-magic value
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x1234_u64,
        4,
    );

    // Exit signal should remain unchanged
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn test_syscon_write_non_zero_offset_ignored() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write magic value to non-zero offset
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000 + 4),
        0x5555_u64,
        4,
    );

    // Exit signal should remain unchanged
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn test_syscon_write_u64_delegates_to_u32() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write poweroff signal as u64 (should delegate to u32)
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0xDEADBEEF_00005555,
        8,
    );

    // Exit signal should be set to 0 (lower 32 bits = 0x5555)
    assert_eq!(exit_signal.peek(), Some(0));
}

#[test]
fn test_syscon_multiple_commands() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // First command: poweroff
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x5555_u64,
        4,
    );
    assert_eq!(exit_signal.peek(), Some(0));

    let _ = exit_signal.take();

    // Second command: failure
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x3333_u64,
        4,
    );
    assert_eq!(exit_signal.peek(), Some(1));
}

#[test]
fn test_syscon_all_magic_values() {
    let base_addr = 0x100000;

    // Test poweroff
    let exit_signal = ExitSignal::new();
    let mut syscon = SysCon::new(PhysAddr::new(base_addr), exit_signal.clone());
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x5555_u64,
        4,
    );
    assert_eq!(exit_signal.peek(), Some(0));

    // Test reset
    let exit_signal = ExitSignal::new();
    let mut syscon = SysCon::new(PhysAddr::new(base_addr), exit_signal.clone());
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x7777_u64,
        4,
    );
    assert_eq!(exit_signal.peek(), Some(0));

    // Test failure
    let exit_signal = ExitSignal::new();
    let mut syscon = SysCon::new(PhysAddr::new(base_addr), exit_signal.clone());
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x3333_u64,
        4,
    );
    assert_eq!(exit_signal.peek(), Some(1));
}
