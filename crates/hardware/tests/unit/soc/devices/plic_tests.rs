//! # PLIC Device Tests
//!
//! Tests for the Platform-Level Interrupt Controller device.

use rvsim_core::SimState;
use rvsim_core::config::Config;

#[test]
fn test_plic_name() {
    let config = Config::default();
    let _cpu = SimState::build(&config, "");
}

#[test]
fn test_plic_device_integration() {
    let config = Config::default();
    let _cpu = SimState::build(&config, "");

    // System should initialize without panicking
}
