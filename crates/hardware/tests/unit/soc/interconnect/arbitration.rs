//! Bus arbitration and device lookup tests.
//!
//! Verifies that the bus correctly caches last-device lookups and
//! handles tick/IRQ propagation.

use riscv_core::soc::devices::clint::Clint;
use riscv_core::soc::interconnect::Bus;
use riscv_core::soc::memory::Memory;
use riscv_core::soc::memory::buffer::DramBuffer;
use std::sync::Arc;

#[test]
fn bus_tick_propagates_to_clint() {
    let mut bus = Bus::new(8, 0);
    let clint = Clint::new(0x200_0000, 1);
    bus.add_device(Box::new(clint));

    // Write mtimecmp = 3
    bus.write_u64(0x200_0000 + 0x4000, 3);

    // Tick 3 times → mtime reaches 3, should trigger timer
    let (t1, _, _) = bus.tick();
    let (t2, _, _) = bus.tick();
    let (t3, _, _) = bus.tick();

    assert!(!t1);
    assert!(!t2);
    assert!(t3, "Timer should fire on 3rd tick");
}

#[test]
fn bus_last_device_cache_hit() {
    let mut bus = Bus::new(8, 0);
    let buf = Arc::new(DramBuffer::new(4096));
    let mem = Memory::new(buf, 0x8000_0000);
    bus.add_device(Box::new(mem));

    // First access primes the cache
    bus.write_u32(0x8000_0000, 0x1234);
    // Second access should hit the cache
    assert_eq!(bus.read_u32(0x8000_0000), 0x1234);
    // Nearby address should also hit cache
    bus.write_u32(0x8000_0004, 0x5678);
    assert_eq!(bus.read_u32(0x8000_0004), 0x5678);
}
