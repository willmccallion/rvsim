//! The memory map the configuration describes is the one the bus builds.

use crate::config::Config;
use crate::system::SystemState;

/// The bus maps every device at the register window the configuration
/// gives it, so validation and the device tree describe the bus as built.
#[test]
fn every_device_sits_at_its_configured_region() {
    let config = Config::minimal();
    let sys = SystemState::build(&config);

    let mut built: Vec<_> = sys
        .uncore
        .bus
        .device_windows()
        .map(|(name, base, size)| (name.to_owned(), base, size))
        .collect();
    let mut expected: Vec<_> = config
        .system
        .device_regions()
        .iter()
        .map(|region| (region.name.to_owned(), region.base, region.size))
        .collect();
    built.sort();
    expected.sort();

    assert_eq!(built, expected);
}

#[test]
fn a_ram_the_host_cannot_map_fails_the_build() {
    let mut config = Config::minimal();
    config.memory.ram_size = usize::MAX;

    let built = crate::Simulator::build(&config, None);

    assert!(
        matches!(built, Err(crate::common::SimError::RamAllocation { bytes: usize::MAX, .. })),
        "{:?}",
        built.err()
    );
}

#[test]
fn a_disk_image_that_cannot_be_read_fails_the_build() {
    const MISSING: &str = "/nonexistent/rvsim-test-disk.img";

    let built = crate::Simulator::build(&Config::minimal(), Some(MISSING));

    assert!(
        matches!(&built, Err(crate::common::SimError::FileRead { path, .. }) if path == MISSING),
        "{:?}",
        built.err()
    );
}
