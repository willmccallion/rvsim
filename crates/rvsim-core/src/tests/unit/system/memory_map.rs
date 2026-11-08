//! The memory map the configuration describes is the one the bus builds.

use crate::Simulator;
use crate::common::PhysAddr;
use crate::common::SimError;
use crate::config::Config;
use crate::system::SystemState;
use crate::tests::support::harness::valid;

/// The bus maps every device at the register window the configuration
/// gives it, so validation and the device tree describe the bus as built.
#[test]
fn every_device_sits_at_its_configured_region() {
    let config = Config::minimal();
    let sys = SystemState::build(&config);

    let mut built: Vec<_> = sys
        .uncore
        .bus
        .devices()
        .map(|device| (device.name().to_owned(), device.region().base, device.region().size))
        .collect();
    let mut expected: Vec<_> = config
        .system
        .device_regions()
        .iter()
        .map(|region| (region.name.to_owned(), region.range.base, region.range.size))
        .collect();
    built.sort();
    expected.sort();

    assert_eq!(built, expected);
}

#[test]
fn a_ram_the_host_cannot_map_fails_the_build() {
    /// More than any host's user address space, and clear of every device.
    const HUGE: u64 = 1 << 62;
    let mut config = Config::minimal();
    config.system.ram_base = PhysAddr::new(HUGE);
    config.memory.ram_size = HUGE;

    let built = Simulator::build(&valid(&config), None);

    assert!(matches!(built, Err(SimError::RamAllocation { bytes: HUGE, .. })), "{:?}", built.err());
}

#[test]
fn a_disk_image_that_cannot_be_read_fails_the_build() {
    const MISSING: &str = "/nonexistent/rvsim-test-disk.img";

    let built = Simulator::build(&valid(&Config::minimal()), Some(MISSING));

    assert!(
        matches!(&built, Err(SimError::FileRead { path, .. }) if path == MISSING),
        "{:?}",
        built.err()
    );
}
