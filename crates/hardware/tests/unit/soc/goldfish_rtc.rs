//! Goldfish RTC unit tests.
//!
//! Verifies basic device identification for the Goldfish real-time clock.

use riscv_core::soc::devices::Device;
use riscv_core::soc::devices::goldfish_rtc::GoldfishRtc;

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
    // The low word of the time should be > 0 (we're running this test
    // some time after the Unix epoch).
    let mut rtc = GoldfishRtc::new(0);
    let time_low = rtc.read_u32(0x0);
    // We don't know the exact time, but it should be positive
    // (nanoseconds since epoch mod 2^32 should be nonzero)
    let _time_high = rtc.read_u32(0x4);
    let time_ns = ((_time_high as u64) << 32) | (time_low as u64);
    assert!(time_ns > 0, "Time since epoch should be > 0");
}
