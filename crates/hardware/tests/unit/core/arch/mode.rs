//! # Privilege Mode Tests
//!
//! This module contains unit tests for RISC-V privilege mode conversions and representations.

use rvsim_core::core::arch::mode::PrivilegeMode;

#[test]
fn test_privilege_mode_from_u8_user() {
    let mode = PrivilegeMode::from_u8(0);
    assert_eq!(mode, PrivilegeMode::User);
}

#[test]
fn test_privilege_mode_from_u8_supervisor() {
    let mode = PrivilegeMode::from_u8(1);
    assert_eq!(mode, PrivilegeMode::Supervisor);
}

#[test]
fn test_privilege_mode_from_u8_machine() {
    let mode = PrivilegeMode::from_u8(3);
    assert_eq!(mode, PrivilegeMode::Machine);
}

#[test]
fn test_privilege_mode_from_u8_invalid_defaults_to_machine() {
    // Invalid privilege values should default to Machine mode
    assert_eq!(PrivilegeMode::from_u8(2), PrivilegeMode::Machine);
    assert_eq!(PrivilegeMode::from_u8(4), PrivilegeMode::Machine);
    assert_eq!(PrivilegeMode::from_u8(255), PrivilegeMode::Machine);
}

#[test]
fn test_privilege_mode_to_u8_user() {
    assert_eq!(PrivilegeMode::User.to_u8(), 0);
}

#[test]
fn test_privilege_mode_to_u8_supervisor() {
    assert_eq!(PrivilegeMode::Supervisor.to_u8(), 1);
}

#[test]
fn test_privilege_mode_to_u8_machine() {
    assert_eq!(PrivilegeMode::Machine.to_u8(), 3);
}

#[test]
fn test_privilege_mode_name_user() {
    assert_eq!(PrivilegeMode::User.name(), "User");
}

#[test]
fn test_privilege_mode_name_supervisor() {
    assert_eq!(PrivilegeMode::Supervisor.name(), "Supervisor");
}

#[test]
fn test_privilege_mode_name_machine() {
    assert_eq!(PrivilegeMode::Machine.name(), "Machine");
}

#[test]
fn test_privilege_mode_display_user() {
    let mode = PrivilegeMode::User;
    assert_eq!(format!("{}", mode), "User");
}

#[test]
fn test_privilege_mode_display_supervisor() {
    let mode = PrivilegeMode::Supervisor;
    assert_eq!(format!("{}", mode), "Supervisor");
}

#[test]
fn test_privilege_mode_display_machine() {
    let mode = PrivilegeMode::Machine;
    assert_eq!(format!("{}", mode), "Machine");
}

#[test]
fn test_privilege_mode_round_trip() {
    // Test round-trip conversion for all valid modes
    for val in [0u8, 1u8, 3u8] {
        let mode = PrivilegeMode::from_u8(val);
        assert_eq!(mode.to_u8(), val);
    }
}

#[test]
fn test_privilege_mode_ordering() {
    // User < Supervisor < Machine
    assert!(PrivilegeMode::User < PrivilegeMode::Supervisor);
    assert!(PrivilegeMode::Supervisor < PrivilegeMode::Machine);
    assert!(PrivilegeMode::User < PrivilegeMode::Machine);
}

#[test]
fn test_privilege_mode_equality() {
    assert_eq!(PrivilegeMode::User, PrivilegeMode::User);
    assert_eq!(PrivilegeMode::Supervisor, PrivilegeMode::Supervisor);
    assert_eq!(PrivilegeMode::Machine, PrivilegeMode::Machine);

    assert_ne!(PrivilegeMode::User, PrivilegeMode::Supervisor);
    assert_ne!(PrivilegeMode::User, PrivilegeMode::Machine);
    assert_ne!(PrivilegeMode::Supervisor, PrivilegeMode::Machine);
}

#[test]
fn test_privilege_mode_clone() {
    let mode = PrivilegeMode::Supervisor;
    let cloned = mode;
    assert_eq!(mode, cloned);
}

#[test]
fn test_privilege_mode_copy() {
    let mode1 = PrivilegeMode::Machine;
    let mode2 = mode1;
    // Both should still be usable after copy
    assert_eq!(mode1, PrivilegeMode::Machine);
    assert_eq!(mode2, PrivilegeMode::Machine);
}
