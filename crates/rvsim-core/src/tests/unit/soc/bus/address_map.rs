//! Bus interconnect unit tests.
//!
//! Verifies bus transit-time calculations. RAM and device routing tests
//! that used the deleted `Memory` device + synchronous bus APIs were
//! superseded by integration tests that drive the packet-based
//! `Bus::handle` path through a `Simulator`.

use crate::soc::bus::Bus;

#[test]
fn transit_time_single_transfer() {
    let bus = Bus::new(8, 2, 1);
    // 8 bytes on 8-byte-wide bus = 1 transfer + 2 latency = 3
    assert_eq!(bus.calculate_transit_time(8), 3);
}

#[test]
fn transit_time_multiple_transfers() {
    let bus = Bus::new(4, 1, 1);
    // 16 bytes on 4-byte bus = 4 transfers + 1 latency = 5
    assert_eq!(bus.calculate_transit_time(16), 5);
}

#[test]
fn transit_time_partial_transfer() {
    let bus = Bus::new(8, 0, 1);
    // 5 bytes on 8-byte bus = ceil(5/8)=1 transfer + 0 latency = 1
    assert_eq!(bus.calculate_transit_time(5), 1);
}

#[test]
fn transit_time_zero_bytes() {
    let bus = Bus::new(8, 1, 1);
    // 0 bytes = 0 transfers + 1 latency = 1
    assert_eq!(bus.calculate_transit_time(0), 1);
}
