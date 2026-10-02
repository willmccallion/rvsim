//! Writes that reach RAM without passing through a hart's store path.
//!
//! DMA and host probes land in the image and are visible to the
//! reservation set and the write log.

use crate::common::{HartId, PhysAddr};
use crate::config::Config;
use crate::sim::memory::write_log::Writer;
use crate::system::SystemState;

const H0: HartId = HartId::new(0);
const H1: HartId = HartId::new(1);
const LINE0: PhysAddr = PhysAddr::new(0x8000_0000);
const LINE1: PhysAddr = PhysAddr::new(0x8000_0040);
const LINE2: PhysAddr = PhysAddr::new(0x8000_0080);

fn two_hart_system() -> SystemState {
    let mut config = Config::default();
    config.system.hart_count = 2;
    SystemState::build(&config, "")
}

#[test]
fn an_external_write_invalidates_every_reservation_it_touches() {
    let mut sys = two_hart_system();
    sys.uncore.memory.reservations_mut().set(H0, LINE0);
    sys.uncore.memory.reservations_mut().set(H1, LINE1);
    sys.uncore.memory.reservations_mut().set(H0, LINE2);

    sys.uncore.memory.write_bytes(Writer::External, PhysAddr::new(0x8000_0030), &[0xaa; 0x20]);

    assert!(!sys.uncore.memory.reservations().check(H0, LINE0), "first line written");
    assert!(!sys.uncore.memory.reservations().check(H1, LINE1), "second line written");
    assert!(
        sys.uncore.memory.reservations().check(H0, LINE2),
        "untouched line keeps its reservation"
    );
}

#[test]
fn an_external_write_is_logged_on_every_line_it_touches() {
    let mut sys = two_hart_system();
    let log = sys.uncore.memory.write_log().expect("two harts share a write log");
    let stamp = log.now();

    sys.uncore.memory.write_bytes(Writer::External, PhysAddr::new(0x8000_0030), &[0xaa; 0x20]);

    let log = sys.uncore.memory.write_log().expect("two harts share a write log");
    assert!(log.written_by_other_since(LINE0, H0, stamp));
    assert!(log.written_by_other_since(LINE1, H0, stamp));
    assert!(!log.written_by_other_since(LINE2, H0, stamp));
}

#[test]
fn a_single_byte_external_write_touches_one_line() {
    let mut sys = two_hart_system();
    sys.uncore.memory.reservations_mut().set(H0, LINE0);
    sys.uncore.memory.reservations_mut().set(H1, LINE1);

    sys.uncore.memory.write_bytes(Writer::External, PhysAddr::new(0x8000_003F), &[0xaa]);

    assert!(!sys.uncore.memory.reservations().check(H0, LINE0));
    assert!(sys.uncore.memory.reservations().check(H1, LINE1));
}

#[test]
fn an_external_write_lands_in_the_image() {
    let mut sys = two_hart_system();

    sys.uncore.memory.write_bytes(Writer::External, PhysAddr::new(0x8000_0030), &[1, 2, 3, 4]);

    assert_eq!(sys.uncore.memory.read(PhysAddr::new(0x8000_0030), 4), Some(0x0403_0201));
}
