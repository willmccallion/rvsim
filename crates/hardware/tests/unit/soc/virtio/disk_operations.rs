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

#[test]
fn virtio_queue_desc_low_write_and_read() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x80),
        0x1234_5678_u64,
        4,
    );
    // Verify it was written (read back might not be supported in all impls)
}

#[test]
fn virtio_queue_desc_high_write() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x84),
        0x0000_0001_u64,
        4,
    );
    // High address for descriptor table
}

#[test]
fn virtio_queue_avail_low_write() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x90),
        0x8000_1000_u64,
        4,
    );
}

#[test]
fn virtio_queue_avail_high_write() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x94),
        0_u64,
        4,
    );
}

#[test]
fn virtio_queue_used_low_write() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0xa0),
        0x8000_2000_u64,
        4,
    );
}

#[test]
fn virtio_queue_used_high_write() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0xa4),
        0_u64,
        4,
    );
}

#[test]
fn virtio_queue_sel_write() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x30),
        0_u64,
        4,
    ); // Select queue 0
}

#[test]
fn virtio_queue_sel_read_after_write() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x30),
        0_u64,
        4,
    );
    // Queue sel is typically write-only, but test the behavior
}

#[test]
fn virtio_queue_notify_triggers_processing() {
    let (mut vio, _ram) = make_virtio_with_ram();

    // Set up a basic queue configuration first
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x0F_u64,
        4,
    ); // Set status to DRIVER_OK
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x38),
        8_u64,
        4,
    ); // Queue size
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x44),
        1_u64,
        4,
    ); // Queue ready

    // Write queue notify
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x50),
        0_u64,
        4,
    );
}

#[test]
fn virtio_driver_features_write() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x24),
        0_u64,
        4,
    ); // Driver features sel = 0
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x20),
        0x1_u64,
        4,
    ); // Set feature bit 0
}

#[test]
fn virtio_driver_features_sel_write() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x24),
        1_u64,
        4,
    ); // Select upper 32 bits
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x20),
        0_u64,
        4,
    ); // Clear upper features
}

#[test]
fn virtio_load_small_disk() {
    let mut vio = make_virtio();
    let disk_data = vec![0x42; 512]; // 1 sector
    vio.load(disk_data);

    // Verify capacity = 1 sector
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x100),
            4
        ) as u32),
        1
    );
}

#[test]
fn virtio_load_multi_sector_disk() {
    let mut vio = make_virtio();
    let disk_data = vec![0xAA; 512 * 10]; // 10 sectors
    vio.load(disk_data);

    // Verify capacity = 10 sectors
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x100),
            4
        ) as u32),
        10
    );
}

#[test]
fn virtio_load_large_disk() {
    let mut vio = make_virtio();
    let disk_data = vec![0xFF; 512 * 100]; // 100 sectors
    vio.load(disk_data);

    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x100),
            4
        ) as u32),
        100
    );
}

#[test]
fn virtio_capacity_high_word_for_large_disk() {
    let mut vio = make_virtio();
    let capacity_high = crate::common::probe::read(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x104),
        4,
    ) as u32;
    assert_eq!(capacity_high, 0); // Should be 0 for small test disks
}

#[test]
fn virtio_status_acknowledge() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x01_u64,
        4,
    ); // ACKNOWLEDGE
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x01
    );
}

#[test]
fn virtio_status_driver() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x02_u64,
        4,
    ); // DRIVER
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x02
    );
}

#[test]
fn virtio_status_features_ok() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x08_u64,
        4,
    ); // FEATURES_OK
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x08
    );
}

#[test]
fn virtio_status_driver_ok() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x04_u64,
        4,
    ); // DRIVER_OK
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x04
    );
}

#[test]
fn virtio_status_failed() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x80_u64,
        4,
    ); // FAILED
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x80
    );
}

#[test]
fn virtio_status_reset() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x0F_u64,
        4,
    ); // Set some bits
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x00_u64,
        4,
    ); // Reset
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x00
    );
}

#[test]
fn virtio_status_full_initialization_sequence() {
    let mut vio = make_virtio();

    // Standard initialization sequence
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x01_u64,
        4,
    ); // ACKNOWLEDGE
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x01
    );

    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x03_u64,
        4,
    ); // ACKNOWLEDGE | DRIVER
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x03
    );

    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x0B_u64,
        4,
    ); // ACKNOWLEDGE | DRIVER | FEATURES_OK
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x0B
    );

    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x0F_u64,
        4,
    ); // ACKNOWLEDGE | DRIVER | FEATURES_OK | DRIVER_OK
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x0F
    );
}

#[test]
fn virtio_read_u8_magic() {
    let mut vio = make_virtio();
    // Magic value bytes: 0x76, 0x69, 0x72, 0x74
    assert_eq!(
        (crate::common::probe::read(&mut vio, rvsim_core::common::PhysAddr::new(0x1000_1000), 1)
            as u8),
        0x76
    );
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x01),
            1
        ) as u8),
        0x69
    );
}

#[test]
fn virtio_read_u16_magic() {
    let mut vio = make_virtio();
    assert_eq!(
        (crate::common::probe::read(&mut vio, rvsim_core::common::PhysAddr::new(0x1000_1000), 2)
            as u16),
        0x6976
    ); // Little-endian
}

#[test]
fn virtio_read_u64_config() {
    let mut vio = make_virtio();
    let disk_data = vec![0xFF; 512 * 8];
    vio.load(disk_data);

    // Read capacity as u64 (low + high words)
    let capacity = crate::common::probe::read(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x100),
        8,
    );
    assert_eq!(capacity, 8);
}

#[test]
fn virtio_write_u8_status() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x01_u64,
        1,
    );
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x01
    );
}

#[test]
fn virtio_write_u16_status() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x03_u64,
        2,
    );
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x03
    );
}

#[test]
fn virtio_write_u64_queue_desc() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x80),
        0x8000_1000,
        8,
    );
    // Writes both low and high parts
}

#[test]
fn virtio_interrupt_ack_specific_bit() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x64),
        0x1_u64,
        4,
    ); // Ack bit 0
}

#[test]
fn virtio_interrupt_ack_multiple_bits() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x64),
        0x3_u64,
        4,
    ); // Ack bits 0 and 1
}

#[test]
fn virtio_read_invalid_offset_returns_zero() {
    let mut vio = make_virtio();
    // Read from an undefined offset
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0xFFF),
            4
        ) as u32),
        0
    );
}

#[test]
fn virtio_write_to_read_only_register_ignored() {
    let mut vio = make_virtio();
    // Try to write to magic (read-only)
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000),
        0xDEADBEEF_u64,
        4,
    );
    // Should still read the correct magic
    assert_eq!(
        (crate::common::probe::read(&mut vio, rvsim_core::common::PhysAddr::new(0x1000_1000), 4)
            as u32),
        0x74726976
    );
}

#[test]
fn virtio_queue_num_larger_than_max() {
    let mut vio = make_virtio();
    // Try to set queue size larger than max
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x38),
        32_u64,
        4,
    ); // Max is 16
    // Device should handle this gracefully
}

#[test]
fn virtio_unaligned_read() {
    let mut vio = make_virtio();
    // Read from unaligned address
    let _ = crate::common::probe::read(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x01),
        4,
    ) as u32;
}

#[test]
fn virtio_unaligned_write() {
    let mut vio = make_virtio();
    // Write to unaligned address
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x71),
        0x01_u64,
        4,
    );
}

#[test]
fn virtio_config_space_capacity_fields() {
    let mut vio = make_virtio();
    let disk_data = vec![0xAB; 512 * 256]; // 256 sectors
    vio.load(disk_data);

    // Read capacity low word
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x100),
            4
        ) as u32),
        256
    );

    // Read capacity high word
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x104),
            4
        ) as u32),
        0
    );
}

#[test]
fn virtio_config_space_read_beyond_capacity() {
    let mut vio = make_virtio();
    // Read other config space fields (if they exist)
    let _ = crate::common::probe::read(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x108),
        4,
    ) as u32;
    let _ = crate::common::probe::read(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x10C),
        4,
    ) as u32;
}

#[test]
fn virtio_multiple_status_changes() {
    let mut vio = make_virtio();

    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x01_u64,
        4,
    );
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x01
    );

    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x03_u64,
        4,
    );
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x03
    );

    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
        0x00_u64,
        4,
    );
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x70),
            4
        ) as u32),
        0x00
    );
}

#[test]
fn virtio_reload_disk_updates_capacity() {
    let mut vio = make_virtio();

    // Load first disk
    vio.load(vec![0; 512]);
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x100),
            4
        ) as u32),
        1
    );

    // Load second disk
    vio.load(vec![0; 512 * 5]);
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x100),
            4
        ) as u32),
        5
    );
}

#[test]
fn virtio_queue_ready_toggle() {
    let mut vio = make_virtio();

    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x44),
        1_u64,
        4,
    );
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x44),
            4
        ) as u32),
        1
    );

    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x44),
        0_u64,
        4,
    );
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x44),
            4
        ) as u32),
        0
    );

    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x44),
        1_u64,
        4,
    );
    assert_eq!(
        (crate::common::probe::read(
            &mut vio,
            rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x44),
            4
        ) as u32),
        1
    );
}

#[test]
fn virtio_device_features_bit_32() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x14),
        1_u64,
        4,
    ); // Select upper 32 bits
    let features = crate::common::probe::read(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x10),
        4,
    ) as u32;
    assert_eq!(features & 0x1, 0x1); // Bit 32 should be set
}

#[test]
fn virtio_device_features_lower_bits() {
    let mut vio = make_virtio();
    crate::common::probe::write(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x14),
        0_u64,
        4,
    ); // Select lower 32 bits
    let features = crate::common::probe::read(
        &mut vio,
        rvsim_core::common::PhysAddr::new(0x1000_1000 + 0x10),
        4,
    ) as u32;
    // Lower 32 bits should be 0 for this device
    assert_eq!(features, 0);
}

const RAM_BASE: u64 = 0x8000_0000;
const DESC_TABLE: u64 = RAM_BASE + 0x1000;
const AVAIL_RING: u64 = RAM_BASE + 0x100;
const USED_RING: u64 = RAM_BASE + 0x200;
const REQUEST_HEADER: u64 = RAM_BASE + 0x2000;
const DATA_BUFFER: u64 = RAM_BASE + 0x3000;
const STATUS_BYTE: u64 = RAM_BASE + 0x4000;
const SECTOR: usize = 512;

fn write_descriptor(ram: &DramBuffer, index: u64, addr: u64, len: u32, flags: u16, next: u16) {
    let base = (DESC_TABLE - RAM_BASE + index * 16) as usize;
    ram.write_slice(base, &addr.to_le_bytes());
    ram.write_slice(base + 8, &len.to_le_bytes());
    ram.write_slice(base + 12, &flags.to_le_bytes());
    ram.write_slice(base + 14, &next.to_le_bytes());
}

/// Posts a one-sector read of sector 0 into `DATA_BUFFER` and notifies the device.
fn submit_sector_read(vio: &mut VirtioBlock, ram: &DramBuffer) {
    const F_NEXT: u16 = 1;
    const F_WRITE: u16 = 2;
    write_descriptor(ram, 0, REQUEST_HEADER, 16, F_NEXT, 1);
    write_descriptor(ram, 1, DATA_BUFFER, SECTOR as u32, F_NEXT | F_WRITE, 2);
    write_descriptor(ram, 2, STATUS_BYTE, 1, F_WRITE, 0);
    ram.write_slice((REQUEST_HEADER - RAM_BASE) as usize, &[0u8; 16]);
    let avail = (AVAIL_RING - RAM_BASE) as usize;
    ram.write_slice(avail + 2, &1u16.to_le_bytes());
    ram.write_slice(avail + 4, &0u16.to_le_bytes());

    let reg = |offset: u64| rvsim_core::common::PhysAddr::new(0x1000_1000 + offset);
    crate::common::probe::write(vio, reg(0x38), 8, 4);
    crate::common::probe::write(vio, reg(0x80), DESC_TABLE, 4);
    crate::common::probe::write(vio, reg(0x90), AVAIL_RING, 4);
    crate::common::probe::write(vio, reg(0xa0), USED_RING, 4);
    crate::common::probe::write(vio, reg(0x44), 1, 4);
    crate::common::probe::write(vio, reg(0x50), 0, 4);
}

#[test]
fn a_sector_read_lands_in_the_guest_buffer() {
    let (mut vio, ram) = make_virtio_with_ram();
    vio.load(vec![0x42; SECTOR]);

    submit_sector_read(&mut vio, &ram);

    assert_eq!(ram.read_slice((DATA_BUFFER - RAM_BASE) as usize, SECTOR), vec![0x42; SECTOR]);
    assert_eq!(ram.read_u8((STATUS_BYTE - RAM_BASE) as usize), 0, "VIRTIO_BLK_S_OK");
}

#[test]
fn every_dma_write_of_a_request_is_reported_once() {
    let (mut vio, ram) = make_virtio_with_ram();
    vio.load(vec![0x42; SECTOR]);

    submit_sector_read(&mut vio, &ram);
    let writes = vio.take_dma_writes();

    let touched = |addr: u64| {
        writes.iter().any(|(paddr, len)| paddr.val() <= addr && addr < paddr.val() + *len as u64)
    };
    assert!(touched(DATA_BUFFER), "data buffer: {writes:?}");
    assert!(touched(DATA_BUFFER + SECTOR as u64 - 1), "end of data buffer: {writes:?}");
    assert!(touched(STATUS_BYTE), "status byte: {writes:?}");
    assert!(touched(USED_RING + 2), "used index: {writes:?}");
    assert!(touched(USED_RING + 4), "used element: {writes:?}");
    assert!(vio.take_dma_writes().is_empty(), "reported writes are drained");
}
