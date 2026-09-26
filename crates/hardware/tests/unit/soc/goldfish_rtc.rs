//! Goldfish RTC unit tests.
//!
//! Verifies basic device identification for the Goldfish real-time clock.

use rvsim_core::soc::devices::Device;
use rvsim_core::soc::devices::goldfish_rtc::GoldfishRtc;

#[test]
fn goldfish_rtc_name() {
    let rtc = GoldfishRtc::new(0x101000);
    assert_eq!(rtc.name(), "GoldfishRTC");
}

#[test]
fn goldfish_rtc_address_range() {
    let rtc = GoldfishRtc::new(0x101000);
    let (base, size) = rtc.address_range();
    assert_eq!(base, 0x101000);
    assert_eq!(size, 0x1000);
}

#[test]
fn goldfish_rtc_read_time_low_nonzero() {
    let mut rtc = GoldfishRtc::new(0);
    let time_low =
        crate::common::probe::read(&mut rtc, rvsim_core::common::PhysAddr::new(0x0), 4) as u32;
    let _time_high =
        crate::common::probe::read(&mut rtc, rvsim_core::common::PhysAddr::new(0x4), 4) as u32;
    let time_ns = ((_time_high as u64) << 32) | (time_low as u64);
    assert!(time_ns > 0, "Time since epoch should be > 0");
}
