use rvsim_core::soc::devices::{Device, Htif};
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

    assert_eq!(htif.read_u8(0), 0);
    assert_eq!(htif.read_u16(0), 0);
    assert_eq!(htif.read_u32(0), 0);
    assert_eq!(htif.read_u64(0), 0);
}

#[test]
fn test_htif_write_u8_u16_ignored() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    htif.write_u8(0, 1);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);

    htif.write_u16(0, 1);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);
}

#[test]
fn test_htif_write_to_non_zero_offset_ignored() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    htif.write_u32(4, 1);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);

    htif.write_u64(8, 1);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);
}

#[test]
fn test_htif_pass() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    htif.write_u64(0, 1);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);
}

#[test]
fn test_htif_fail() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    // value 3 is test number 1 (3 >> 1)
    htif.write_u64(0, 3);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 1);

    // value 5 is test number 2 (5 >> 1)
    htif.write_u64(0, 5);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 2);
}

#[test]
fn test_htif_zero_ignored() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    htif.write_u64(0, 0);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0xff);
}

#[test]
fn test_htif_even_non_zero_stored_raw() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    htif.write_u64(0, 42);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 42);
}

#[test]
fn test_htif_write_u32() {
    let exit_signal = Arc::new(AtomicU64::new(0xff));
    let mut htif = Htif::new(0x1000, exit_signal.clone());

    // U32 pass
    htif.write_u32(0, 1);
    assert_eq!(exit_signal.load(Ordering::Relaxed), 0);
}
