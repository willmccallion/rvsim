use crate::soc::devices::{Device, Htif};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn test_htif_name() {
    let exit_signal = Arc::new(AtomicU64::new(0));
    let htif = Htif::new(0x1000, exit_signal, Vec::new());
    assert_eq!(htif.name(), "HTIF");
}

#[test]
fn test_htif_address_range() {
    let exit_signal = Arc::new(AtomicU64::new(0));
    let htif = Htif::new(0x1000, exit_signal, Vec::new());
    assert_eq!(htif.address_range(), (0x1000, 16));
}

#[test]
fn test_htif_read_returns_zero() {
    let exit_signal = Arc::new(AtomicU64::new(0));
    let mut htif = Htif::new(0x1000, exit_signal, Vec::new());

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
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1_u64, 1);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1_u64, 2);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);
}

#[test]
fn test_htif_write_to_non_zero_offset_ignored() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new());

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
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1, 8);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);
}

#[test]
fn test_htif_fail() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new());

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
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 0, 8);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);
}

/// An even value is the address of a syscall block, not an exit; one
/// outside RAM is dropped and the run goes on.
#[test]
fn an_even_tohost_value_is_a_syscall_block_not_an_exit() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 42, 8);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);
}

#[test]
fn test_htif_write_u32() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new());

    // U32 pass
    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1_u64, 4);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);
}

/// `fromhost` is part of a checkpoint: a restored device answers the
/// target's poll as the saved one would have.
#[test]
fn fromhost_survives_a_checkpoint() {
    let exit_signal = Arc::new(AtomicU64::new(0));
    let mut htif = Htif::new(0x1000, exit_signal, Vec::new());
    let state = serde_json::json!({ "fromhost": 5, "host_files_open": 0 });

    htif.restore(&state).expect("the state restores");

    assert_eq!(
        crate::tests::support::probe::read(&mut htif, crate::common::PhysAddr::new(0x1008), 8),
        5
    );
    assert_eq!(htif.checkpoint(), Some(state));
}

/// A checkpoint taken with host files open cannot be restored into a
/// device that has none of them.
#[test]
fn a_checkpoint_with_host_files_open_is_refused() {
    let exit_signal = Arc::new(AtomicU64::new(0));
    let htif = Htif::new(0x1000, exit_signal, Vec::new());
    let state = serde_json::json!({ "fromhost": 0, "host_files_open": 2 });

    let refused = htif.check_restore(&state);

    assert!(refused.is_err(), "{refused:?}");
}
