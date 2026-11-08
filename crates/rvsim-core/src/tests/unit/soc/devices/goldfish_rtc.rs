//! Goldfish RTC unit tests.
//!
//! Verifies basic device identification for the Goldfish real-time clock.

use crate::system::simulator::ProbeWidth;

use crate::common::PhysAddr;
use crate::soc::devices::Device;
use crate::soc::devices::goldfish_rtc::GoldfishRtc;
use crate::tests::support::probe;

/// 2026-01-01T00:00:00Z.
const EPOCH_NS: u64 = 1_767_225_600 * 1_000_000_000;
const CLOCK_MHZ: u64 = 2_400;

#[test]
fn goldfish_rtc_name() {
    let rtc = GoldfishRtc::new(
        PhysAddr::new(0x101000),
        EPOCH_NS,
        std::num::NonZero::new(CLOCK_MHZ).unwrap(),
    );
    assert_eq!(rtc.name(), "GoldfishRTC");
}

#[test]
fn goldfish_rtc_address_range() {
    let rtc = GoldfishRtc::new(
        PhysAddr::new(0x101000),
        EPOCH_NS,
        std::num::NonZero::new(CLOCK_MHZ).unwrap(),
    );
    let region = rtc.region();
    let (base, size) = (region.base.val(), region.size);
    assert_eq!(base, 0x101000);
    assert_eq!(size, 0x1000);
}

#[test]
fn goldfish_rtc_read_time_low_nonzero() {
    let mut rtc =
        GoldfishRtc::new(PhysAddr::new(0), EPOCH_NS, std::num::NonZero::new(CLOCK_MHZ).unwrap());
    let time_low = probe::read(&mut rtc, PhysAddr::new(0x0), ProbeWidth::B4) as u32;
    let _time_high = probe::read(&mut rtc, PhysAddr::new(0x4), ProbeWidth::B4) as u32;
    let time_ns = (u64::from(_time_high) << 32) | u64::from(time_low);
    assert!(time_ns > 0, "Time since epoch should be > 0");
}

#[test]
fn the_clock_reads_the_epoch_at_cycle_zero_and_advances_with_simulated_time() {
    let rtc =
        GoldfishRtc::new(PhysAddr::new(0), EPOCH_NS, std::num::NonZero::new(CLOCK_MHZ).unwrap());

    assert_eq!(rtc.time_ns(0), EPOCH_NS);
    assert_eq!(rtc.time_ns(2_400), EPOCH_NS + 1_000, "2400 cycles at 2.4 GHz is a microsecond");
    assert_eq!(rtc.time_ns(2_400_000_000), EPOCH_NS + 1_000_000_000, "and 2.4 G cycles a second");
}

#[test]
fn two_devices_built_alike_read_the_same_clock() {
    let a =
        GoldfishRtc::new(PhysAddr::new(0), EPOCH_NS, std::num::NonZero::new(CLOCK_MHZ).unwrap());
    let b =
        GoldfishRtc::new(PhysAddr::new(0), EPOCH_NS, std::num::NonZero::new(CLOCK_MHZ).unwrap());

    assert_eq!(a.time_ns(123_456_789), b.time_ns(123_456_789));
}
