use crate::soc::devices::{Device, Htif};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn test_htif_name() {
    let exit_signal = Arc::new(AtomicU64::new(0));
    let htif = Htif::new(0x1000, exit_signal);
    assert_eq!(htif.name(), "HTIF");
}

#[test]
fn test_htif_address_range() {
    let exit_signal = Arc::new(AtomicU64::new(0));
    let htif = Htif::new(0x1000, exit_signal);
    assert_eq!(htif.address_range(), (0x1000, 16));
}

#[test]
fn test_htif_read_returns_zero() {
    let exit_signal = Arc::new(AtomicU64::new(0));
    let mut htif = Htif::new(0x1000, exit_signal);

    assert_eq!(
        crate::tests::support::probe::read(&mut htif, crate::common::PhysAddr::new(0x1000), 1)
            as u8,
        0
    );
    assert_eq!(
        crate::tests::support::probe::read(&mut htif, crate::common::PhysAddr::new(0x1000), 2)
            as u16,
        0
    );
    assert_eq!(
        crate::tests::support::probe::read(&mut htif, crate::common::PhysAddr::new(0x1000), 4)
            as u32,
        0
    );
    assert_eq!(
        crate::tests::support::probe::read(&mut htif, crate::common::PhysAddr::new(0x1000), 8),
        0
    );
}

#[test]
fn test_htif_write_u8_u16_ignored() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1_u64, 1);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1_u64, 2);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);
}

#[test]
fn test_htif_write_to_non_zero_offset_ignored() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    crate::tests::support::probe::write(
        &mut htif,
        crate::common::PhysAddr::new(0x1000 + 4),
        1_u64,
        4,
    );
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000 + 8), 1, 8);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);
}

#[test]
fn test_htif_pass() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1, 8);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);
}

#[test]
fn test_htif_fail() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    // value 3 is test number 1 (3 >> 1)
    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 3, 8);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 1);

    // value 5 is test number 2 (5 >> 1)
    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 5, 8);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 2);
}

#[test]
fn test_htif_zero_ignored() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 0, 8);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);
}

#[test]
fn test_htif_even_non_zero_stored_raw() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 42, 8);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 42);
}

#[test]
fn test_htif_write_u32() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    // U32 pass
    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1_u64, 4);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);
}
