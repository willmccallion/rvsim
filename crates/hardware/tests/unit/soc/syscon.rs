//! # SysCon Device Tests
//!
//! This module contains unit tests for the System Controller (SysCon) device,
//! which provides power management and reset functionality through memory-mapped
//! registers.

use rvsim_core::soc::devices::Device;
use rvsim_core::soc::devices::syscon::SysCon;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Helper function to create a test SysCon device.
fn create_test_syscon() -> (SysCon, Arc<AtomicU64>) {
    let base_addr = 0x100000;
    let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
    let syscon = SysCon::new(base_addr, exit_signal.clone());
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
    let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
    let syscon = SysCon::new(base_addr, exit_signal);

    let (base, size) = syscon.address_range();
    assert_eq!(base, base_addr);
    assert_eq!(size, 0x1000);
}

#[test]
fn test_syscon_read_u8_returns_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(syscon.read_u8(0), 0);
    assert_eq!(syscon.read_u8(0x100), 0);
}

#[test]
fn test_syscon_read_u16_returns_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(syscon.read_u16(0), 0);
    assert_eq!(syscon.read_u16(0x100), 0);
}

#[test]
fn test_syscon_read_u32_returns_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(syscon.read_u32(0), 0);
    assert_eq!(syscon.read_u32(0x100), 0);
}

#[test]
fn test_syscon_read_u64_returns_zero() {
    let (mut syscon, _) = create_test_syscon();
    assert_eq!(syscon.read_u64(0), 0);
    assert_eq!(syscon.read_u64(0x100), 0);
}

#[test]
fn test_syscon_write_u8_ignored() {
    let (mut syscon, exit_signal) = create_test_syscon();

    syscon.write_u8(0, 0x55);
    // Exit signal should remain unchanged
    assert_eq!(exit_signal.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn test_syscon_write_u16_ignored() {
    let (mut syscon, exit_signal) = create_test_syscon();

    syscon.write_u16(0, 0x5555);
    // Exit signal should remain unchanged
    assert_eq!(exit_signal.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn test_syscon_poweroff_signal() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write poweroff magic value (0x5555) to offset 0
    syscon.write_u32(0, 0x5555);

    // Exit signal should be set to 0
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);
}

#[test]
fn test_syscon_reset_signal() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write reset magic value (0x7777) to offset 0
    syscon.write_u32(0, 0x7777);

    // Exit signal should be set to 0 (simulated as exit)
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);
}

#[test]
fn test_syscon_failure_signal() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write failure magic value (0x3333) to offset 0
    syscon.write_u32(0, 0x3333);

    // Exit signal should be set to 1 (failure)
    assert_eq!(exit_signal.load(Ordering::Relaxed), 1);
}

#[test]
fn test_syscon_write_non_magic_value_ignored() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write non-magic value
    syscon.write_u32(0, 0x1234);

    // Exit signal should remain unchanged
    assert_eq!(exit_signal.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn test_syscon_write_non_zero_offset_ignored() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write magic value to non-zero offset
    syscon.write_u32(4, 0x5555);

    // Exit signal should remain unchanged
    assert_eq!(exit_signal.load(Ordering::Relaxed), u64::MAX);
}

#[test]
fn test_syscon_write_u64_delegates_to_u32() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // Write poweroff signal as u64 (should delegate to u32)
    syscon.write_u64(0, 0xDEADBEEF_00005555);

    // Exit signal should be set to 0 (lower 32 bits = 0x5555)
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);
}

#[test]
fn test_syscon_multiple_commands() {
    let (mut syscon, exit_signal) = create_test_syscon();

    // First command: poweroff
    syscon.write_u32(0, 0x5555);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);

    // Reset exit signal
    exit_signal.store(u64::MAX, Ordering::Relaxed);

    // Second command: failure
    syscon.write_u32(0, 0x3333);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 1);
}

#[test]
fn test_syscon_all_magic_values() {
    let base_addr = 0x100000;

    // Test poweroff
    let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
    let mut syscon = SysCon::new(base_addr, exit_signal.clone());
    syscon.write_u32(0, 0x5555);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);

    // Test reset
    let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
    let mut syscon = SysCon::new(base_addr, exit_signal.clone());
    syscon.write_u32(0, 0x7777);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);

    // Test failure
    let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
    let mut syscon = SysCon::new(base_addr, exit_signal.clone());
    syscon.write_u32(0, 0x3333);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 1);
}
