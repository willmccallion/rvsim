//! Bus interconnect unit tests.
//!
//! Verifies device registration, address routing, read/write operations,
//! transit time calculation, and valid address checks.

use riscv_core::soc::interconnect::Bus;
use riscv_core::soc::memory::Memory;
use riscv_core::soc::memory::buffer::DramBuffer;
use std::sync::Arc;

fn make_bus_with_ram(size: usize, base: u64) -> Bus {
    let mut bus = Bus::new(8, 1);
    let buf = Arc::new(DramBuffer::new(size));
    let mem = Memory::new(buf, base);
    bus.add_device(Box::new(mem));
    bus
}

// ══════════════════════════════════════════════════════════
// 1. Transit time calculation
// ══════════════════════════════════════════════════════════

#[test]
fn transit_time_single_transfer() {
    let bus = Bus::new(8, 2);
    // 8 bytes on 8-byte-wide bus = 1 transfer + 2 latency = 3
    assert_eq!(bus.calculate_transit_time(8), 3);
}

#[test]
fn transit_time_multiple_transfers() {
    let bus = Bus::new(4, 1);
    // 16 bytes on 4-byte bus = 4 transfers + 1 latency = 5
    assert_eq!(bus.calculate_transit_time(16), 5);
}

#[test]
fn transit_time_partial_transfer() {
    let bus = Bus::new(8, 0);
    // 5 bytes on 8-byte bus = ceil(5/8)=1 transfer + 0 latency = 1
    assert_eq!(bus.calculate_transit_time(5), 1);
}

#[test]
fn transit_time_zero_bytes() {
    let bus = Bus::new(8, 1);
    // 0 bytes = 0 transfers + 1 latency = 1
    assert_eq!(bus.calculate_transit_time(0), 1);
}

// ══════════════════════════════════════════════════════════
// 2. RAM read/write
// ══════════════════════════════════════════════════════════

#[test]
fn ram_write_u8_read_u8() {
    let mut bus = make_bus_with_ram(4096, 0x8000_0000);
    bus.write_u8(0x8000_0000, 0xAB);
    assert_eq!(bus.read_u8(0x8000_0000), 0xAB);
}

#[test]
fn ram_write_u16_read_u16() {
    let mut bus = make_bus_with_ram(4096, 0x8000_0000);
    bus.write_u16(0x8000_0000, 0xBEEF);
    assert_eq!(bus.read_u16(0x8000_0000), 0xBEEF);
}

#[test]
fn ram_write_u32_read_u32() {
    let mut bus = make_bus_with_ram(4096, 0x8000_0000);
    bus.write_u32(0x8000_0000, 0xDEAD_BEEF);
    assert_eq!(bus.read_u32(0x8000_0000), 0xDEAD_BEEF);
}

#[test]
fn ram_write_u64_read_u64() {
    let mut bus = make_bus_with_ram(4096, 0x8000_0000);
    bus.write_u64(0x8000_0000, 0xCAFE_BABE_DEAD_BEEF);
    assert_eq!(bus.read_u64(0x8000_0000), 0xCAFE_BABE_DEAD_BEEF);
}

#[test]
fn ram_initial_value_zero() {
    let mut bus = make_bus_with_ram(4096, 0x8000_0000);
    assert_eq!(bus.read_u64(0x8000_0000), 0);
}

// ══════════════════════════════════════════════════════════
// 3. Valid address checking
// ══════════════════════════════════════════════════════════

#[test]
fn is_valid_address_in_ram() {
    let bus = make_bus_with_ram(4096, 0x8000_0000);
    assert!(bus.is_valid_address(0x8000_0000));
    assert!(bus.is_valid_address(0x8000_0FFF));
}

#[test]
fn is_valid_address_outside_ram() {
    let bus = make_bus_with_ram(4096, 0x8000_0000);
    assert!(!bus.is_valid_address(0x7FFF_FFFF));
    assert!(!bus.is_valid_address(0x8000_1000));
}

// ══════════════════════════════════════════════════════════
// 4. No device returns 0
// ══════════════════════════════════════════════════════════

#[test]
fn read_unmapped_address_returns_zero() {
    let mut bus = Bus::new(8, 0);
    assert_eq!(bus.read_u8(0x1000), 0);
    assert_eq!(bus.read_u16(0x1000), 0);
    assert_eq!(bus.read_u32(0x1000), 0);
    assert_eq!(bus.read_u64(0x1000), 0);
}

// ══════════════════════════════════════════════════════════
// 5. load_binary_at
// ══════════════════════════════════════════════════════════

#[test]
fn load_binary_at_writes_data() {
    let mut bus = make_bus_with_ram(4096, 0x8000_0000);
    let data = [0xDE, 0xAD, 0xBE, 0xEF];
    bus.load_binary_at(&data, 0x8000_0000);

    assert_eq!(bus.read_u8(0x8000_0000), 0xDE);
    assert_eq!(bus.read_u8(0x8000_0001), 0xAD);
    assert_eq!(bus.read_u8(0x8000_0002), 0xBE);
    assert_eq!(bus.read_u8(0x8000_0003), 0xEF);
}

// ══════════════════════════════════════════════════════════
// 6. Multiple devices sorted by base address
// ══════════════════════════════════════════════════════════

#[test]
fn multiple_devices_routed_correctly() {
    let mut bus = Bus::new(8, 0);

    let buf1 = Arc::new(DramBuffer::new(256));
    let mem1 = Memory::new(buf1, 0x1000);
    bus.add_device(Box::new(mem1));

    let buf2 = Arc::new(DramBuffer::new(256));
    let mem2 = Memory::new(buf2, 0x2000);
    bus.add_device(Box::new(mem2));

    bus.write_u32(0x1000, 0xAAAA);
    bus.write_u32(0x2000, 0xBBBB);

    assert_eq!(bus.read_u32(0x1000), 0xAAAA);
    assert_eq!(bus.read_u32(0x2000), 0xBBBB);
}

// ══════════════════════════════════════════════════════════
// 7. RAM info
// ══════════════════════════════════════════════════════════

#[test]
fn get_ram_info_returns_some() {
    let mut bus = make_bus_with_ram(4096, 0x8000_0000);
    let info = bus.get_ram_info();
    assert!(info.is_some());
    let (_ptr, base, end) = info.unwrap();
    assert_eq!(base, 0x8000_0000);
    assert_eq!(end, 0x8000_0000 + 4096);
}

#[test]
fn get_ram_info_none_when_no_ram() {
    let mut bus = Bus::new(8, 0);
    assert!(bus.get_ram_info().is_none());
}
