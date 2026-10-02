//! Goldfish RTC unit tests.
//!
//! Verifies basic device identification for the Goldfish real-time clock.

use crate::soc::devices::Device;
use crate::soc::devices::goldfish_rtc::GoldfishRtc;

/// 2026-01-01T00:00:00Z.
const EPOCH_NS: u64 = 1_767_225_600 * 1_000_000_000;
const CLOCK_MHZ: u64 = 2_400;

#[test]
fn goldfish_rtc_name() {
    let rtc = GoldfishRtc::new(0x101000, EPOCH_NS, CLOCK_MHZ);
    assert_eq!(rtc.name(), "GoldfishRTC");
}

#[test]
fn goldfish_rtc_address_range() {
    let rtc = GoldfishRtc::new(0x101000, EPOCH_NS, CLOCK_MHZ);
    let (base, size) = rtc.address_range();
    assert_eq!(base, 0x101000);
    assert_eq!(size, 0x1000);
}

#[test]
fn goldfish_rtc_read_time_low_nonzero() {
    let mut rtc = GoldfishRtc::new(0, EPOCH_NS, CLOCK_MHZ);
    let time_low =
        crate::tests::support::probe::read(&mut rtc, crate::common::PhysAddr::new(0x0), 4) as u32;
    let _time_high =
        crate::tests::support::probe::read(&mut rtc, crate::common::PhysAddr::new(0x4), 4) as u32;
    let time_ns = ((_time_high as u64) << 32) | (time_low as u64);
    assert!(time_ns > 0, "Time since epoch should be > 0");
}

#[test]
fn the_clock_reads_the_epoch_at_cycle_zero_and_advances_with_simulated_time() {
    let rtc = GoldfishRtc::new(0, EPOCH_NS, CLOCK_MHZ);

    assert_eq!(rtc.time_ns(0), EPOCH_NS);
    assert_eq!(rtc.time_ns(2_400), EPOCH_NS + 1_000, "2400 cycles at 2.4 GHz is a microsecond");
    assert_eq!(rtc.time_ns(2_400_000_000), EPOCH_NS + 1_000_000_000, "and 2.4 G cycles a second");
}

#[test]
fn two_devices_built_alike_read_the_same_clock() {
    let a = GoldfishRtc::new(0, EPOCH_NS, CLOCK_MHZ);
    let b = GoldfishRtc::new(0, EPOCH_NS, CLOCK_MHZ);

    assert_eq!(a.time_ns(123_456_789), b.time_ns(123_456_789));
}
