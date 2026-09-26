//! Writes that reach RAM without passing through a hart's store path.
//!
//! DMA and host probes must still be visible to the reservation set and
//! the write log.

use rvsim_core::common::{HartId, PhysAddr};
use rvsim_core::config::Config;
use rvsim_core::sim::SimState;

const H0: HartId = HartId::new(0);
const H1: HartId = HartId::new(1);
const LINE0: PhysAddr = PhysAddr::new(0x8000_0000);
const LINE1: PhysAddr = PhysAddr::new(0x8000_0040);
const LINE2: PhysAddr = PhysAddr::new(0x8000_0080);

fn two_hart_system() -> SimState {
    let mut config = Config::default();
    config.system.hart_count = 2;
    SimState::build(&config, "")
}

#[test]
fn external_write_range_invalidates_every_reservation_it_touches() {
    let mut sys = two_hart_system();
    sys.shared.reservations.set(H0, LINE0);
    sys.shared.reservations.set(H1, LINE1);
    sys.shared.reservations.set(H0, LINE2);

    sys.shared.record_external_write_range(PhysAddr::new(0x8000_0030), 0x20);

    assert!(!sys.shared.reservations.check(H0, LINE0), "first line written");
    assert!(!sys.shared.reservations.check(H1, LINE1), "second line written");
    assert!(sys.shared.reservations.check(H0, LINE2), "untouched line keeps its reservation");
}

#[test]
fn external_write_range_is_logged_on_every_line_it_touches() {
    let mut sys = two_hart_system();
    let log = sys.shared.write_log.as_ref().expect("two harts share a write log");
    let stamp = log.now();

    sys.shared.record_external_write_range(PhysAddr::new(0x8000_0030), 0x20);

    let log = sys.shared.write_log.as_ref().expect("two harts share a write log");
    assert!(log.written_by_other_since(LINE0, H0, stamp));
    assert!(log.written_by_other_since(LINE1, H0, stamp));
    assert!(!log.written_by_other_since(LINE2, H0, stamp));
}

#[test]
fn a_single_byte_external_write_touches_one_line() {
    let mut sys = two_hart_system();
    sys.shared.reservations.set(H0, LINE0);
    sys.shared.reservations.set(H1, LINE1);

    sys.shared.record_external_write_range(PhysAddr::new(0x8000_003F), 1);

    assert!(!sys.shared.reservations.check(H0, LINE0));
    assert!(sys.shared.reservations.check(H1, LINE1));
}
