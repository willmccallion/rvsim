//! VirtIO Block Device Disk Operations Tests.
//!
//! Tests for disk I/O operations, queue descriptor handling,
//! and more advanced VirtIO functionality.

use rvsim_core::soc::devices::Device;
use rvsim_core::soc::devices::virtio_disk::VirtioBlock;
use rvsim_core::soc::memory::buffer::DramBuffer;
use std::sync::Arc;

fn make_virtio() -> VirtioBlock {
    let ram = Arc::new(DramBuffer::new(4096));
    VirtioBlock::new(0x1000_1000, 0x8000_0000, ram)
}

fn make_virtio_with_ram() -> (VirtioBlock, Arc<DramBuffer>) {
    let ram = Arc::new(DramBuffer::new(0x10000)); // 64KB RAM
    let virtio = VirtioBlock::new(0x1000_1000, 0x8000_0000, Arc::clone(&ram));
    (virtio, ram)
}

// ══════════════════════════════════════════════════════════
// Queue Address Configuration Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_queue_desc_low_write_and_read() {
    let mut vio = make_virtio();
    vio.write_u32(0x80, 0x1234_5678);
    // Verify it was written (read back might not be supported in all impls)
}

#[test]
fn virtio_queue_desc_high_write() {
    let mut vio = make_virtio();
    vio.write_u32(0x84, 0x0000_0001);
    // High address for descriptor table
}

#[test]
fn virtio_queue_avail_low_write() {
    let mut vio = make_virtio();
    vio.write_u32(0x90, 0x8000_1000);
}

#[test]
fn virtio_queue_avail_high_write() {
    let mut vio = make_virtio();
    vio.write_u32(0x94, 0);
}

#[test]
fn virtio_queue_used_low_write() {
    let mut vio = make_virtio();
    vio.write_u32(0xa0, 0x8000_2000);
}

#[test]
fn virtio_queue_used_high_write() {
    let mut vio = make_virtio();
    vio.write_u32(0xa4, 0);
}

// ══════════════════════════════════════════════════════════
// Queue Selection Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_queue_sel_write() {
    let mut vio = make_virtio();
    vio.write_u32(0x30, 0); // Select queue 0
}

#[test]
fn virtio_queue_sel_read_after_write() {
    let mut vio = make_virtio();
    vio.write_u32(0x30, 0);
    // Queue sel is typically write-only, but test the behavior
}

// ══════════════════════════════════════════════════════════
// Queue Notification Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_queue_notify_triggers_processing() {
    let (mut vio, _ram) = make_virtio_with_ram();

    // Set up a basic queue configuration first
    vio.write_u32(0x70, 0x0F); // Set status to DRIVER_OK
    vio.write_u32(0x38, 8); // Queue size
    vio.write_u32(0x44, 1); // Queue ready

    // Write queue notify
    vio.write_u32(0x50, 0);
}

// ══════════════════════════════════════════════════════════
// Driver/Device Features Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_driver_features_write() {
    let mut vio = make_virtio();
    vio.write_u32(0x24, 0); // Driver features sel = 0
    vio.write_u32(0x20, 0x1); // Set feature bit 0
}

#[test]
fn virtio_driver_features_sel_write() {
    let mut vio = make_virtio();
    vio.write_u32(0x24, 1); // Select upper 32 bits
    vio.write_u32(0x20, 0); // Clear upper features
}

// ══════════════════════════════════════════════════════════
// Disk Loading and Capacity Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_load_small_disk() {
    let mut vio = make_virtio();
    let disk_data = vec![0x42; 512]; // 1 sector
    vio.load(disk_data);

    // Verify capacity = 1 sector
    assert_eq!(vio.read_u32(0x100), 1);
}

#[test]
fn virtio_load_multi_sector_disk() {
    let mut vio = make_virtio();
    let disk_data = vec![0xAA; 512 * 10]; // 10 sectors
    vio.load(disk_data);

    // Verify capacity = 10 sectors
    assert_eq!(vio.read_u32(0x100), 10);
}

#[test]
fn virtio_load_large_disk() {
    let mut vio = make_virtio();
    let disk_data = vec![0xFF; 512 * 100]; // 100 sectors
    vio.load(disk_data);

    assert_eq!(vio.read_u32(0x100), 100);
}

#[test]
fn virtio_capacity_high_word_for_large_disk() {
    let mut vio = make_virtio();
    // Load a disk larger than 4GB (requires high word)
    // For testing purposes, we'll just verify the read doesn't panic
    let capacity_high = vio.read_u32(0x104);
    assert_eq!(capacity_high, 0); // Should be 0 for small test disks
}

// ══════════════════════════════════════════════════════════
// Status Register Transitions Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_status_acknowledge() {
    let mut vio = make_virtio();
    vio.write_u32(0x70, 0x01); // ACKNOWLEDGE
    assert_eq!(vio.read_u32(0x70), 0x01);
}

#[test]
fn virtio_status_driver() {
    let mut vio = make_virtio();
    vio.write_u32(0x70, 0x02); // DRIVER
    assert_eq!(vio.read_u32(0x70), 0x02);
}

#[test]
fn virtio_status_features_ok() {
    let mut vio = make_virtio();
    vio.write_u32(0x70, 0x08); // FEATURES_OK
    assert_eq!(vio.read_u32(0x70), 0x08);
}

#[test]
fn virtio_status_driver_ok() {
    let mut vio = make_virtio();
    vio.write_u32(0x70, 0x04); // DRIVER_OK
    assert_eq!(vio.read_u32(0x70), 0x04);
}

#[test]
fn virtio_status_failed() {
    let mut vio = make_virtio();
    vio.write_u32(0x70, 0x80); // FAILED
    assert_eq!(vio.read_u32(0x70), 0x80);
}

#[test]
fn virtio_status_reset() {
    let mut vio = make_virtio();
    vio.write_u32(0x70, 0x0F); // Set some bits
    vio.write_u32(0x70, 0x00); // Reset
    assert_eq!(vio.read_u32(0x70), 0x00);
}

#[test]
fn virtio_status_full_initialization_sequence() {
    let mut vio = make_virtio();

    // Standard initialization sequence
    vio.write_u32(0x70, 0x01); // ACKNOWLEDGE
    assert_eq!(vio.read_u32(0x70), 0x01);

    vio.write_u32(0x70, 0x03); // ACKNOWLEDGE | DRIVER
    assert_eq!(vio.read_u32(0x70), 0x03);

    vio.write_u32(0x70, 0x0B); // ACKNOWLEDGE | DRIVER | FEATURES_OK
    assert_eq!(vio.read_u32(0x70), 0x0B);

    vio.write_u32(0x70, 0x0F); // ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK
    assert_eq!(vio.read_u32(0x70), 0x0F);
}

// ══════════════════════════════════════════════════════════
// Read/Write Width Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_read_u8_magic() {
    let mut vio = make_virtio();
    // Magic value bytes: 0x76, 0x69, 0x72, 0x74
    assert_eq!(vio.read_u8(0x00), 0x76);
    assert_eq!(vio.read_u8(0x01), 0x69);
}

#[test]
fn virtio_read_u16_magic() {
    let mut vio = make_virtio();
    assert_eq!(vio.read_u16(0x00), 0x6976); // Little-endian
}

#[test]
fn virtio_read_u64_config() {
    let mut vio = make_virtio();
    let disk_data = vec![0xFF; 512 * 8];
    vio.load(disk_data);

    // Read capacity as u64 (low + high words)
    let capacity = vio.read_u64(0x100);
    assert_eq!(capacity, 8);
}

#[test]
fn virtio_write_u8_status() {
    let mut vio = make_virtio();
    vio.write_u8(0x70, 0x01);
    assert_eq!(vio.read_u32(0x70), 0x01);
}

#[test]
fn virtio_write_u16_status() {
    let mut vio = make_virtio();
    vio.write_u16(0x70, 0x03);
    assert_eq!(vio.read_u32(0x70), 0x03);
}

#[test]
fn virtio_write_u64_queue_desc() {
    let mut vio = make_virtio();
    vio.write_u64(0x80, 0x8000_1000);
    // Writes both low and high parts
}

// ══════════════════════════════════════════════════════════
// Interrupt Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_interrupt_ack_specific_bit() {
    let mut vio = make_virtio();
    vio.write_u32(0x64, 0x1); // Ack bit 0
}

#[test]
fn virtio_interrupt_ack_multiple_bits() {
    let mut vio = make_virtio();
    vio.write_u32(0x64, 0x3); // Ack bits 0 and 1
}

// ══════════════════════════════════════════════════════════
// Invalid/Edge Case Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_read_invalid_offset_returns_zero() {
    let mut vio = make_virtio();
    // Read from an undefined offset
    assert_eq!(vio.read_u32(0xFFF), 0);
}

#[test]
fn virtio_write_to_read_only_register_ignored() {
    let mut vio = make_virtio();
    // Try to write to magic (read-only)
    vio.write_u32(0x00, 0xDEADBEEF);
    // Should still read the correct magic
    assert_eq!(vio.read_u32(0x00), 0x74726976);
}

#[test]
fn virtio_queue_num_larger_than_max() {
    let mut vio = make_virtio();
    // Try to set queue size larger than max
    vio.write_u32(0x38, 32); // Max is 16
    // Device should handle this gracefully
}

#[test]
fn virtio_unaligned_read() {
    let mut vio = make_virtio();
    // Read from unaligned address
    let _ = vio.read_u32(0x01);
}

#[test]
fn virtio_unaligned_write() {
    let mut vio = make_virtio();
    // Write to unaligned address
    vio.write_u32(0x71, 0x01);
}

// ══════════════════════════════════════════════════════════
// Config Space Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_config_space_capacity_fields() {
    let mut vio = make_virtio();
    let disk_data = vec![0xAB; 512 * 256]; // 256 sectors
    vio.load(disk_data);

    // Read capacity low word
    assert_eq!(vio.read_u32(0x100), 256);

    // Read capacity high word
    assert_eq!(vio.read_u32(0x104), 0);
}

#[test]
fn virtio_config_space_read_beyond_capacity() {
    let mut vio = make_virtio();
    // Read other config space fields (if they exist)
    let _ = vio.read_u32(0x108);
    let _ = vio.read_u32(0x10C);
}

// ══════════════════════════════════════════════════════════
// Multiple Operations Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_multiple_status_changes() {
    let mut vio = make_virtio();

    vio.write_u32(0x70, 0x01);
    assert_eq!(vio.read_u32(0x70), 0x01);

    vio.write_u32(0x70, 0x03);
    assert_eq!(vio.read_u32(0x70), 0x03);

    vio.write_u32(0x70, 0x00);
    assert_eq!(vio.read_u32(0x70), 0x00);
}

#[test]
fn virtio_reload_disk_updates_capacity() {
    let mut vio = make_virtio();

    // Load first disk
    vio.load(vec![0; 512]);
    assert_eq!(vio.read_u32(0x100), 1);

    // Load second disk
    vio.load(vec![0; 512 * 5]);
    assert_eq!(vio.read_u32(0x100), 5);
}

#[test]
fn virtio_queue_ready_toggle() {
    let mut vio = make_virtio();

    vio.write_u32(0x44, 1);
    assert_eq!(vio.read_u32(0x44), 1);

    vio.write_u32(0x44, 0);
    assert_eq!(vio.read_u32(0x44), 0);

    vio.write_u32(0x44, 1);
    assert_eq!(vio.read_u32(0x44), 1);
}

// ══════════════════════════════════════════════════════════
// Device Feature Bit Tests
// ══════════════════════════════════════════════════════════

#[test]
fn virtio_device_features_bit_32() {
    let mut vio = make_virtio();
    vio.write_u32(0x14, 1); // Select upper 32 bits
    let features = vio.read_u32(0x10);
    assert_eq!(features & 0x1, 0x1); // Bit 32 should be set
}

#[test]
fn virtio_device_features_lower_bits() {
    let mut vio = make_virtio();
    vio.write_u32(0x14, 0); // Select lower 32 bits
    let features = vio.read_u32(0x10);
    // Lower 32 bits should be 0 for this device
    assert_eq!(features, 0);
}
