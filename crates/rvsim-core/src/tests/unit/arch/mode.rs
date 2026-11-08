//! Privilege modes: their encoding, names and order.

use crate::isa::privileged::PrivilegeMode;

#[test]
fn each_mode_has_its_spec_encoding_and_name() {
    let modes = [
        (PrivilegeMode::User, 0, "User"),
        (PrivilegeMode::Supervisor, 1, "Supervisor"),
        (PrivilegeMode::Machine, 3, "Machine"),
    ];

    for (mode, encoding, name) in modes {
        assert_eq!(PrivilegeMode::from_u8(encoding), mode);
        assert_eq!(mode.to_u8(), encoding);
        assert_eq!(mode.name(), name);
        assert_eq!(mode.to_string(), name);
    }
}

#[test]
fn reserved_and_out_of_range_values_decode_as_machine_mode() {
    for encoding in [2, 4, 255] {
        assert_eq!(PrivilegeMode::from_u8(encoding), PrivilegeMode::Machine, "{encoding}");
    }
}

#[test]
fn modes_order_user_below_supervisor_below_machine() {
    assert!(PrivilegeMode::User < PrivilegeMode::Supervisor);
    assert!(PrivilegeMode::Supervisor < PrivilegeMode::Machine);
}
