//! # PLIC Device Tests
//!
//! Tests for the Platform-Level Interrupt Controller device.

use crate::config::Config;
use crate::system::SystemState;

#[test]
fn test_plic_name() {
    let config = Config::minimal();
    let _cpu = SystemState::build(&config, "");
}

#[test]
fn test_plic_device_integration() {
    let config = Config::minimal();
    let _cpu = SystemState::build(&config, "");

    // System should initialize without panicking
}
