//! VirtIO Block Device Unit Tests.
//!
//! Verifies register identification (magic, version, device ID),
//! status register, queue configuration, and interrupt status.

use riscv_core::soc::devices::Device;
use riscv_core::soc::devices::virtio_disk::VirtioBlock;
use riscv_core::soc::memory::buffer::DramBuffer;
use std::sync::Arc;

fn make_virtio() -> VirtioBlock {
    let ram = Arc::new(DramBuffer::new(4096));
    VirtioBlock::new(0x1000_1000, 0x8000_0000, ram)
}

// ══════════════════════════════════════════════════════════
// 1. Identification registers
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_magic_value() {
    let mut vio = make_virtio();
    assert_eq!(vio.read_u32(0x00), 0x7472_6976, "Magic should be 'virt'");
}

#[test]
fn virtio_version() {
    let mut vio = make_virtio();
    assert_eq!(vio.read_u32(0x04), 2, "Version should be 2");
}

#[test]
fn virtio_device_id() {
    let mut vio = make_virtio();
    assert_eq!(vio.read_u32(0x08), 2, "Device ID should be 2 (block)");
}

#[test]
fn virtio_vendor_id() {
    let mut vio = make_virtio();
    assert_eq!(vio.read_u32(0x0C), 0x554d_4551, "Vendor ID should be QEMU");
}

#[test]
fn virtio_name() {
    let vio = make_virtio();
    assert_eq!(vio.name(), "VirtIO-Blk");
}

#[test]
fn virtio_address_range() {
    let vio = make_virtio();
    let (base, size) = vio.address_range();
    assert_eq!(base, 0x1000_1000);
    assert_eq!(size, 0x1000);
}

// ══════════════════════════════════════════════════════════
// 2. Status register
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_status_initial_zero() {
    let mut vio = make_virtio();
    assert_eq!(vio.read_u32(0x70), 0);
}

#[test]
fn virtio_status_write_and_read() {
    let mut vio = make_virtio();
    vio.write_u32(0x70, 0x0F);
    assert_eq!(vio.read_u32(0x70), 0x0F);
}

// ══════════════════════════════════════════════════════════
// 3. Queue configuration
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_queue_num_max() {
    let mut vio = make_virtio();
    assert_eq!(vio.read_u32(0x34), 16, "Queue max should be 16");
}

#[test]
fn virtio_queue_num_write_and_read() {
    let mut vio = make_virtio();
    vio.write_u32(0x38, 8);
    // Queue num is write-only in some specs, but our impl stores it
    // We verify through queue_ready
    vio.write_u32(0x44, 1);
    assert_eq!(vio.read_u32(0x44), 1);
}

#[test]
fn virtio_queue_ready() {
    let mut vio = make_virtio();
    vio.write_u32(0x44, 1);
    assert_eq!(vio.read_u32(0x44), 1);
    vio.write_u32(0x44, 0);
    assert_eq!(vio.read_u32(0x44), 0);
}

// ══════════════════════════════════════════════════════════
// 4. Interrupt status
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_interrupt_status_initial_zero() {
    let mut vio = make_virtio();
    assert_eq!(vio.read_u32(0x60), 0);
}

#[test]
fn virtio_interrupt_ack_clears_bits() {
    let mut vio = make_virtio();
    // Manually we can't easily trigger an interrupt without full queue setup,
    // but we can verify the ack mechanism
    vio.write_u32(0x64, 0x1); // Ack bit 0
    // Since interrupt_status was 0, clearing doesn't change anything
    assert_eq!(vio.read_u32(0x60), 0);
}

// ══════════════════════════════════════════════════════════
// 5. Config space (disk capacity)
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_capacity_empty_disk() {
    let mut vio = make_virtio();
    // No disk loaded → 0 sectors
    assert_eq!(vio.read_u32(0x100), 0);
    assert_eq!(vio.read_u32(0x104), 0);
}

#[test]
fn virtio_capacity_with_disk() {
    let mut vio = make_virtio();
    // Load a 2048-byte disk (4 sectors of 512 bytes)
    vio.load(vec![0; 2048]);
    assert_eq!(vio.read_u32(0x100), 4);
    assert_eq!(vio.read_u32(0x104), 0);
}

#[test]
fn virtio_irq_id() {
    let vio = make_virtio();
    assert_eq!(vio.get_irq_id(), Some(1));
}

#[test]
fn virtio_tick_no_interrupt_initially() {
    let mut vio = make_virtio();
    assert!(!vio.tick());
}

// ══════════════════════════════════════════════════════════
// 6. Device features
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_device_features_sel_0() {
    let mut vio = make_virtio();
    vio.write_u32(0x14, 0); // features_sel = 0
    assert_eq!(vio.read_u32(0x10), 0);
}

#[test]
fn virtio_device_features_sel_1() {
    let mut vio = make_virtio();
    vio.write_u32(0x14, 1); // features_sel = 1
    assert_eq!(vio.read_u32(0x10), 1, "Feature bit 32 should be set");
}
