//! # SysCon Device Tests
//!
//! This module contains unit tests for the System Controller (SysCon) device,
//! which provides power management and reset functionality through memory-mapped
//! registers.

use crate::system::simulator::ProbeWidth;

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
fn the_syscon_is_named_syscon() {
    let (syscon, _) = create_test_syscon();
    assert_eq!(syscon.name(), "SysCon");
}

#[test]
fn the_syscon_region_is_4_kib_at_its_base() {
    let base_addr = 0x100000;
    let exit_signal = ExitSignal::new();
    let syscon = SysCon::new(PhysAddr::new(base_addr), exit_signal);

    let region = syscon.region();
    let (base, size) = (region.base.val(), region.size);
    assert_eq!(base, base_addr);
    assert_eq!(size, 0x1000);
}

#[test]
fn byte_reads_return_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(
        (crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000),
            ProbeWidth::B1
        ) as u8),
        0
    );
    assert_eq!(
        (crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000 + 0x100),
            ProbeWidth::B1
        ) as u8),
        0
    );
}

#[test]
fn halfword_reads_return_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(
        (crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000),
            ProbeWidth::B2
        ) as u16),
        0
    );
    assert_eq!(
        (crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000 + 0x100),
            ProbeWidth::B2
        ) as u16),
        0
    );
}

#[test]
fn word_reads_return_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(
        (crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000),
            ProbeWidth::B4
        ) as u32),
        0
    );
    assert_eq!(
        (crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000 + 0x100),
            ProbeWidth::B4
        ) as u32),
        0
    );
}

#[test]
fn doubleword_reads_return_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(
        crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000),
            ProbeWidth::B8
        ),
        0
    );
    assert_eq!(
        crate::tests::support::probe::read(
            &mut syscon,
            crate::common::PhysAddr::new(0x100000 + 0x100),
            ProbeWidth::B8
        ),
        0
    );
}

#[test]
fn a_byte_write_signals_no_exit() {
    let (mut syscon, exit_signal) = create_test_syscon();

    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x55_u64,
        ProbeWidth::B1,
    );
    // Exit signal should remain unchanged
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn a_halfword_poweroff_write_signals_no_exit() {
    let (mut syscon, exit_signal) = create_test_syscon();

    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x5555_u64,
        ProbeWidth::B2,
    );
    // Exit signal should remain unchanged
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn poweroff_exits_with_code_zero() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write poweroff magic value (0x5555) to offset 0
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x5555_u64,
        ProbeWidth::B4,
    );

    // Exit signal should be set to 0
    assert_eq!(exit_signal.peek(), Some(0));
}

#[test]
fn reset_exits_with_code_zero() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write reset magic value (0x7777) to offset 0
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x7777_u64,
        ProbeWidth::B4,
    );

    // Exit signal should be set to 0 (simulated as exit)
    assert_eq!(exit_signal.peek(), Some(0));
}

#[test]
fn failure_exits_with_code_one() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write failure magic value (0x3333) to offset 0
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x3333_u64,
        ProbeWidth::B4,
    );

    // Exit signal should be set to 1 (failure)
    assert_eq!(exit_signal.peek(), Some(1));
}

#[test]
fn a_word_that_is_no_command_signals_no_exit() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write non-magic value
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x1234_u64,
        ProbeWidth::B4,
    );

    // Exit signal should remain unchanged
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn a_command_past_offset_zero_signals_no_exit() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write magic value to non-zero offset
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000 + 4),
        0x5555_u64,
        ProbeWidth::B4,
    );

    // Exit signal should remain unchanged
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn a_doubleword_write_acts_on_its_low_word() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write poweroff signal as u64 (should delegate to u32)
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0xDEADBEEF_00005555,
        ProbeWidth::B8,
    );

    // Exit signal should be set to 0 (lower 32 bits = 0x5555)
    assert_eq!(exit_signal.peek(), Some(0));
}

#[test]
fn a_later_command_signals_again_after_the_exit_is_taken() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // First command: poweroff
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x5555_u64,
        ProbeWidth::B4,
    );
    assert_eq!(exit_signal.peek(), Some(0));

    let _ = exit_signal.take();

    // Second command: failure
    crate::tests::support::probe::write(
        &mut syscon,
        crate::common::PhysAddr::new(0x100000),
        0x3333_u64,
        ProbeWidth::B4,
    );
    assert_eq!(exit_signal.peek(), Some(1));
}
