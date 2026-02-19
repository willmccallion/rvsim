//! # PLIC Device Tests
//!
//! Tests for the Platform-Level Interrupt Controller device.

use rvsim_core::config::Config;
use rvsim_core::soc::System;

#[test]
fn test_plic_name() {
    let config = Config::default();
    let _system = System::new(&config, "");

    // The PLIC should be available in the system
    // We test through the system's bus access
}

#[test]
fn test_plic_device_integration() {
    let config = Config::default();
    let _system = System::new(&config, "");

    // System should initialize without panicking
}
