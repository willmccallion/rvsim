//! # Privilege Mode Tests
//!
//! This module contains unit tests for RISC-V privilege mode conversions and representations.

use crate::isa::privileged::PrivilegeMode;

#[test]
fn zero_decodes_as_user_mode() {
    let mode = PrivilegeMode::from_u8(0);
    assert_eq!(mode, PrivilegeMode::User);
}

#[test]
fn one_decodes_as_supervisor_mode() {
    let mode = PrivilegeMode::from_u8(1);
    assert_eq!(mode, PrivilegeMode::Supervisor);
}

#[test]
fn three_decodes_as_machine_mode() {
    let mode = PrivilegeMode::from_u8(3);
    assert_eq!(mode, PrivilegeMode::Machine);
}

#[test]
fn reserved_and_out_of_range_values_decode_as_machine_mode() {
    // Invalid privilege values should default to Machine mode
    assert_eq!(PrivilegeMode::from_u8(2), PrivilegeMode::Machine);
    assert_eq!(PrivilegeMode::from_u8(4), PrivilegeMode::Machine);
    assert_eq!(PrivilegeMode::from_u8(255), PrivilegeMode::Machine);
}

#[test]
fn user_mode_encodes_as_zero() {
    assert_eq!(PrivilegeMode::User.to_u8(), 0);
}

#[test]
fn supervisor_mode_encodes_as_one() {
    assert_eq!(PrivilegeMode::Supervisor.to_u8(), 1);
}

#[test]
fn machine_mode_encodes_as_three() {
    assert_eq!(PrivilegeMode::Machine.to_u8(), 3);
}

#[test]
fn user_mode_is_named_user() {
    assert_eq!(PrivilegeMode::User.name(), "User");
}

#[test]
fn supervisor_mode_is_named_supervisor() {
    assert_eq!(PrivilegeMode::Supervisor.name(), "Supervisor");
}

#[test]
fn machine_mode_is_named_machine() {
    assert_eq!(PrivilegeMode::Machine.name(), "Machine");
}

#[test]
fn user_mode_displays_as_user() {
    let mode = PrivilegeMode::User;
    assert_eq!(format!("{}", mode), "User");
}

#[test]
fn supervisor_mode_displays_as_supervisor() {
    let mode = PrivilegeMode::Supervisor;
    assert_eq!(format!("{}", mode), "Supervisor");
}

#[test]
fn machine_mode_displays_as_machine() {
    let mode = PrivilegeMode::Machine;
    assert_eq!(format!("{}", mode), "Machine");
}

#[test]
fn valid_encodings_round_trip_through_privilege_mode() {
    // Test round-trip conversion for all valid modes
    for val in [0u8, 1u8, 3u8] {
        let mode = PrivilegeMode::from_u8(val);
        assert_eq!(mode.to_u8(), val);
    }
}

#[test]
fn modes_order_user_below_supervisor_below_machine() {
    // User < Supervisor < Machine
    assert!(PrivilegeMode::User < PrivilegeMode::Supervisor);
    assert!(PrivilegeMode::Supervisor < PrivilegeMode::Machine);
    assert!(PrivilegeMode::User < PrivilegeMode::Machine);
}

#[test]
fn each_mode_equals_only_itself() {
    assert_eq!(PrivilegeMode::User, PrivilegeMode::User);
    assert_eq!(PrivilegeMode::Supervisor, PrivilegeMode::Supervisor);
    assert_eq!(PrivilegeMode::Machine, PrivilegeMode::Machine);

    assert_ne!(PrivilegeMode::User, PrivilegeMode::Supervisor);
    assert_ne!(PrivilegeMode::User, PrivilegeMode::Machine);
    assert_ne!(PrivilegeMode::Supervisor, PrivilegeMode::Machine);
}

#[test]
fn a_copied_mode_equals_the_original() {
    let mode = PrivilegeMode::Supervisor;
    let cloned = mode;
    assert_eq!(mode, cloned);
}

#[test]
fn a_mode_stays_usable_after_being_copied() {
    let mode1 = PrivilegeMode::Machine;
    let mode2 = mode1;
    // Both should still be usable after copy
    assert_eq!(mode1, PrivilegeMode::Machine);
    assert_eq!(mode2, PrivilegeMode::Machine);
}
