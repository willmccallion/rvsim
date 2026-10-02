//! RAM is a zeroed image at a base address that hands out bounds-checked
//! slices of itself.

use crate::common::PhysAddr;
use crate::sim::memory::Ram;

const BASE: u64 = 0x8000_0000;

#[test]
fn a_new_image_is_zeroed_and_sized() {
    let ram = Ram::new(BASE, 256);

    assert_eq!(ram.base(), BASE);
    assert_eq!(ram.size(), 256);
    assert!(ram.bytes().iter().all(|&byte| byte == 0));
}

#[test]
fn a_slice_inside_the_image_is_returned_at_its_address() {
    let mut ram = Ram::new(BASE, 256);

    ram.get_mut(PhysAddr::new(BASE + 10), 4).expect("inside").copy_from_slice(&[1, 2, 3, 4]);

    assert_eq!(ram.get(PhysAddr::new(BASE + 10), 4), Some(&[1, 2, 3, 4][..]));
    assert_eq!(&ram.bytes()[10..14], &[1, 2, 3, 4]);
}

#[test]
fn a_slice_that_leaves_the_image_is_refused() {
    let mut ram = Ram::new(BASE, 256);

    assert!(ram.get(PhysAddr::new(BASE - 1), 1).is_none(), "before the base");
    assert!(ram.get(PhysAddr::new(BASE + 255), 2).is_none(), "past the end");
    assert!(ram.get_mut(PhysAddr::new(u64::MAX), 1).is_none(), "wrapping address");
    assert_eq!(ram.get(PhysAddr::new(BASE + 252), 4).map(<[u8]>::len), Some(4), "last bytes");
}

#[test]
fn contains_covers_exactly_the_image() {
    let ram = Ram::new(BASE, 64);

    assert!(ram.contains(PhysAddr::new(BASE), 64));
    assert!(ram.contains(PhysAddr::new(BASE + 63), 1));
    assert!(!ram.contains(PhysAddr::new(BASE + 63), 2));
    assert!(!ram.contains(PhysAddr::new(BASE), u64::MAX));
}

#[test]
fn the_whole_image_can_be_overwritten() {
    let mut ram = Ram::new(BASE, 64);

    ram.bytes_mut().fill(0x5a);

    assert_eq!(ram.get(PhysAddr::new(BASE + 63), 1), Some(&[0x5a][..]));
}

#[test]
fn a_large_image_is_usable_at_its_last_byte() {
    let size = 64 << 20;
    let mut ram = Ram::new(0, size);

    ram.get_mut(PhysAddr::new(size as u64 - 1), 1).expect("last byte")[0] = 0xff;

    assert_eq!(ram.size(), size as u64);
    assert_eq!(ram.get(PhysAddr::new(size as u64 - 1), 1), Some(&[0xff][..]));
}

#[test]
fn an_empty_image_contains_nothing() {
    let ram = Ram::new(BASE, 0);

    assert_eq!(ram.size(), 0);
    assert!(ram.bytes().is_empty());
    assert!(ram.get(PhysAddr::new(BASE), 1).is_none());
}
