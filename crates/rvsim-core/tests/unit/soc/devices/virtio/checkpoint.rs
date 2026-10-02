//! Sectors the guest wrote survive a checkpoint, and a checkpoint only
//! restores onto the disk image it was taken on.

use super::dma_timing::{
    DATA, MMIO, Request, SECTOR, device_with_a_queued, disk_image, notify, place,
};
use rvsim_core::sim::events::EventQueue;
use rvsim_core::soc::devices::Device;
use rvsim_core::soc::devices::virtio_disk::VirtioBlock;

const WRITTEN: u8 = 0xab;

/// A device that has written `WRITTEN` over `SECTOR`.
fn device_after_a_write() -> VirtioBlock {
    let (mut device, mut memory) = device_with_a_queued(Request::Write);
    place(&mut memory, DATA, &[WRITTEN; 512]);
    notify(&mut device, &mut memory, &mut EventQueue::new());
    device.drain(&mut memory);
    device
}

fn fresh_device(image: Vec<u8>) -> VirtioBlock {
    let mut device = VirtioBlock::new(MMIO);
    device.load(image);
    device
}

#[test]
fn a_written_sector_is_restored_onto_a_fresh_copy_of_the_image() {
    let state = device_after_a_write().checkpoint().expect("the disk has state");
    let mut restored = fresh_device(disk_image());

    restored.restore(&state).expect("the image is the one the checkpoint was taken on");

    let written = restored.state().written;
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].sector, SECTOR);
    assert_eq!(written[0].data, "ab".repeat(512));
    assert_eq!(restored.checkpoint(), Some(state));
}

#[test]
fn a_checkpoint_is_refused_by_a_different_image() {
    let state = device_after_a_write().checkpoint().expect("the disk has state");
    let mut other_image = disk_image();
    other_image[0] ^= 1;
    let mut restored = fresh_device(other_image);
    let before = restored.checkpoint();

    let result = restored.restore(&state);

    assert!(result.is_err());
    assert_eq!(restored.checkpoint(), before, "a refused restore changes nothing");
}

#[test]
fn a_written_sector_past_the_end_of_the_disk_is_refused() {
    let mut state = device_after_a_write().state();
    state.written[0].sector = 4;
    let mut restored = fresh_device(disk_image());

    assert!(restored.set_state(&state).is_err());
}
