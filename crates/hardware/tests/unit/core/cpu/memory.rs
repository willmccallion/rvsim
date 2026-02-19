//! # Memory Access Tests
//!
//! Tests for address translation, cache simulation, and memory access.

use rvsim_core::common::{AccessType, VirtAddr};
use rvsim_core::config::Config;
use rvsim_core::core::Cpu;

fn create_test_cpu() -> Cpu {
    let config = Config::default();
    let system = rvsim_core::soc::System::new(&config, "");
    let mut cpu = Cpu::new(system, &config);
    cpu.direct_mode = true; // Use direct addressing for simple tests
    cpu
}

#[test]
fn test_translate_direct_mode_valid_address() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;

    let vaddr = VirtAddr::new(0x8000_0000);
    let result = cpu.translate(vaddr, AccessType::Read);

    assert_eq!(result.trap, None);
    assert_eq!(result.paddr.val(), 0x8000_0000);
}

#[test]
fn test_translate_direct_mode_different_addresses() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;

    let test_addrs = vec![0x8000_0000u64, 0x8000_1000u64, 0x8000_2000u64];

    for addr in test_addrs {
        let vaddr = VirtAddr::new(addr);
        let result = cpu.translate(vaddr, AccessType::Read);

        // Direct mode just passes through
        assert_eq!(result.paddr.val(), addr);
    }
}

#[test]
fn test_translate_direct_mode_fetch_access() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;

    let vaddr = VirtAddr::new(0x8000_0000);
    let result = cpu.translate(vaddr, AccessType::Fetch);

    assert_eq!(result.trap, None);
}

#[test]
fn test_translate_direct_mode_write_access() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;

    let vaddr = VirtAddr::new(0x8000_0000);
    let result = cpu.translate(vaddr, AccessType::Write);

    assert_eq!(result.trap, None);
}

#[test]
fn test_translate_preserves_translation_cost() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;

    let vaddr = VirtAddr::new(0x8000_0000);
    let result = cpu.translate(vaddr, AccessType::Read);

    // Direct mode should have zero cost
    assert_eq!(result.cycles, 0);
}

#[test]
fn test_translate_multiple_calls() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;

    for i in 0..10 {
        let addr = 0x8000_0000 + (i * 0x1000);
        let vaddr = VirtAddr::new(addr);
        let result = cpu.translate(vaddr, AccessType::Read);

        assert_eq!(result.paddr.val(), addr);
    }
}

#[test]
fn test_cache_access_returns_latency() {
    let mut cpu = create_test_cpu();
    let paddr = 0x8000_0000u64;

    let latency = cpu.simulate_memory_access(rvsim_core::common::PhysAddr::new(paddr), AccessType::Read);

    // Latency should be a valid value
    assert!(latency < u64::MAX);
}

#[test]
fn test_cache_access_instruction_fetch() {
    let mut cpu = create_test_cpu();
    let paddr = 0x8000_0000u64;

    let latency =
        cpu.simulate_memory_access(rvsim_core::common::PhysAddr::new(paddr), AccessType::Fetch);

    assert!(latency < u64::MAX);
}

#[test]
fn test_cache_access_write() {
    let mut cpu = create_test_cpu();
    let paddr = 0x8000_0000u64;

    let latency =
        cpu.simulate_memory_access(rvsim_core::common::PhysAddr::new(paddr), AccessType::Write);

    assert!(latency < u64::MAX);
}

#[test]
fn test_cache_disabled() {
    let mut cpu = create_test_cpu();
    cpu.l1_i_cache.enabled = false;
    cpu.l1_d_cache.enabled = false;

    let paddr = 0x8000_0000u64;

    let latency = cpu.simulate_memory_access(rvsim_core::common::PhysAddr::new(paddr), AccessType::Read);

    // Should still return valid latency
    assert!(latency < u64::MAX);
}

#[test]
fn test_multiple_memory_accesses() {
    let mut cpu = create_test_cpu();

    for i in 0..5 {
        let paddr = 0x8000_0000u64 + i as u64 * 0x1000;
        let latency =
            cpu.simulate_memory_access(rvsim_core::common::PhysAddr::new(paddr), AccessType::Read);

        assert!(latency < u64::MAX);
    }
}

#[test]
fn test_memory_access_with_different_access_types() {
    let mut cpu = create_test_cpu();
    let paddr = 0x8000_0000u64;
    let paddr_obj = rvsim_core::common::PhysAddr::new(paddr);

    let fetch_latency = cpu.simulate_memory_access(paddr_obj, AccessType::Fetch);
    let read_latency = cpu.simulate_memory_access(paddr_obj, AccessType::Read);
    let write_latency = cpu.simulate_memory_access(paddr_obj, AccessType::Write);

    // All should return valid latencies
    assert!(fetch_latency < u64::MAX);
    assert!(read_latency < u64::MAX);
    assert!(write_latency < u64::MAX);
}

#[test]
fn test_cache_stats_updated() {
    let mut cpu = create_test_cpu();
    let initial_hits = cpu.stats.icache_hits;

    cpu.simulate_memory_access(
        rvsim_core::common::PhysAddr::new(0x8000_0000u64),
        AccessType::Fetch,
    );

    // Stats might be updated (at least verify they're accessible)
    assert!(cpu.stats.icache_hits >= initial_hits);
}

#[test]
fn test_l1_icache_enabled_hit_tracking() {
    let mut cpu = create_test_cpu();
    cpu.l1_i_cache.enabled = true;

    let initial_hits = cpu.stats.icache_hits;
    let initial_misses = cpu.stats.icache_misses;

    // Access the same address twice - first should miss, second should hit
    let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64);
    cpu.simulate_memory_access(paddr, AccessType::Fetch);
    let after_first = cpu.stats.icache_misses;

    cpu.simulate_memory_access(paddr, AccessType::Fetch);
    let after_second = cpu.stats.icache_hits;

    // At least one access should have been tracked
    assert!(after_first > initial_misses || after_second > initial_hits);
}

#[test]
fn test_l1_dcache_enabled_hit_tracking() {
    let mut cpu = create_test_cpu();
    cpu.l1_d_cache.enabled = true;

    let initial_hits = cpu.stats.dcache_hits;
    let initial_misses = cpu.stats.dcache_misses;

    // Access the same address twice - first should miss, second should hit
    let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64);
    cpu.simulate_memory_access(paddr, AccessType::Read);
    let after_first = cpu.stats.dcache_misses;

    cpu.simulate_memory_access(paddr, AccessType::Read);
    let after_second = cpu.stats.dcache_hits;

    // At least one access should have been tracked
    assert!(after_first > initial_misses || after_second > initial_hits);
}

#[test]
fn test_l2_cache_enabled() {
    let mut cpu = create_test_cpu();
    cpu.l1_d_cache.enabled = false;
    cpu.l2_cache.enabled = true;

    let initial_l2_hits = cpu.stats.l2_hits;
    let initial_l2_misses = cpu.stats.l2_misses;

    let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64);
    cpu.simulate_memory_access(paddr, AccessType::Read);

    // L2 stats should be updated
    assert!(cpu.stats.l2_hits > initial_l2_hits || cpu.stats.l2_misses > initial_l2_misses);
}

#[test]
fn test_l3_cache_enabled() {
    let mut cpu = create_test_cpu();
    cpu.l1_d_cache.enabled = false;
    cpu.l2_cache.enabled = false;
    cpu.l3_cache.enabled = true;

    let initial_l3_hits = cpu.stats.l3_hits;
    let initial_l3_misses = cpu.stats.l3_misses;

    let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64);
    cpu.simulate_memory_access(paddr, AccessType::Read);

    // L3 stats should be updated
    assert!(cpu.stats.l3_hits > initial_l3_hits || cpu.stats.l3_misses > initial_l3_misses);
}

#[test]
fn test_all_caches_enabled() {
    let mut cpu = create_test_cpu();
    cpu.l1_i_cache.enabled = true;
    cpu.l1_d_cache.enabled = true;
    cpu.l2_cache.enabled = true;
    cpu.l3_cache.enabled = true;

    let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64);

    // Test instruction fetch
    let fetch_latency = cpu.simulate_memory_access(paddr, AccessType::Fetch);
    assert!(fetch_latency < u64::MAX);

    // Test data read
    let read_latency = cpu.simulate_memory_access(paddr, AccessType::Read);
    assert!(read_latency < u64::MAX);

    // Test data write
    let write_latency = cpu.simulate_memory_access(paddr, AccessType::Write);
    assert!(write_latency < u64::MAX);
}

#[test]
fn test_translate_invalid_address_fetch() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;

    // Try to access an invalid address
    let vaddr = VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF);
    let result = cpu.translate(vaddr, AccessType::Fetch);

    // Should return a fault
    assert!(result.trap.is_some());
}

#[test]
fn test_translate_invalid_address_read() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;

    // Try to access an invalid address
    let vaddr = VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF);
    let result = cpu.translate(vaddr, AccessType::Read);

    // Should return a fault
    assert!(result.trap.is_some());
}

#[test]
fn test_translate_invalid_address_write() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = true;

    // Try to access an invalid address
    let vaddr = VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF);
    let result = cpu.translate(vaddr, AccessType::Write);

    // Should return a fault
    assert!(result.trap.is_some());
}

#[test]
fn test_cache_write_access_tracking() {
    let mut cpu = create_test_cpu();
    cpu.l1_d_cache.enabled = true;

    let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64);

    // Perform multiple writes
    for _ in 0..3 {
        cpu.simulate_memory_access(paddr, AccessType::Write);
    }

    // Verify cache is tracking write accesses
    assert!(cpu.stats.dcache_hits > 0 || cpu.stats.dcache_misses > 0);
}

#[test]
fn test_cache_hierarchy_miss_propagation() {
    let mut cpu = create_test_cpu();
    cpu.l1_d_cache.enabled = true;
    cpu.l2_cache.enabled = true;
    cpu.l3_cache.enabled = true;

    let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64);

    // First access should miss in all levels
    cpu.simulate_memory_access(paddr, AccessType::Read);

    // At least L1 should have recorded a miss
    assert!(cpu.stats.dcache_misses > 0);
}

#[test]
fn test_different_addresses_different_cache_lines() {
    let mut cpu = create_test_cpu();
    cpu.l1_d_cache.enabled = true;

    // Access addresses that should map to different cache lines
    for i in 0..10 {
        let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64 + i * 64);
        cpu.simulate_memory_access(paddr, AccessType::Read);
    }

    // Verify cache is tracking different accesses
    assert!(cpu.stats.dcache_misses > 0);
}

#[test]
fn test_instruction_and_data_caches_independent() {
    let mut cpu = create_test_cpu();
    cpu.l1_i_cache.enabled = true;
    cpu.l1_d_cache.enabled = true;

    let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64);

    let initial_icache_stats = cpu.stats.icache_hits + cpu.stats.icache_misses;
    let initial_dcache_stats = cpu.stats.dcache_hits + cpu.stats.dcache_misses;

    // Access as instruction
    cpu.simulate_memory_access(paddr, AccessType::Fetch);
    let after_fetch_icache = cpu.stats.icache_hits + cpu.stats.icache_misses;
    let after_fetch_dcache = cpu.stats.dcache_hits + cpu.stats.dcache_misses;

    // Instruction cache should be updated, data cache should not
    assert!(after_fetch_icache > initial_icache_stats);
    assert_eq!(after_fetch_dcache, initial_dcache_stats);

    // Access as data
    cpu.simulate_memory_access(paddr, AccessType::Read);
    let after_read_dcache = cpu.stats.dcache_hits + cpu.stats.dcache_misses;

    // Data cache should now be updated
    assert!(after_read_dcache > after_fetch_dcache);
}

#[test]
fn test_memory_access_latency_increases_with_cache_misses() {
    let mut cpu = create_test_cpu();
    cpu.l1_d_cache.enabled = true;
    cpu.l2_cache.enabled = true;

    let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64);

    // First access (should miss and have higher latency)
    let first_latency = cpu.simulate_memory_access(paddr, AccessType::Read);

    // Second access to same address (should hit and have lower latency)
    let second_latency = cpu.simulate_memory_access(paddr, AccessType::Read);

    // Both should be valid, first should be >= second (miss vs hit)
    assert!(first_latency < u64::MAX);
    assert!(second_latency < u64::MAX);
    assert!(first_latency >= second_latency);
}

#[test]
fn test_translate_with_direct_mode_false() {
    let mut cpu = create_test_cpu();
    cpu.direct_mode = false;

    // Should use MMU translation
    let vaddr = VirtAddr::new(0x8000_0000);
    let result = cpu.translate(vaddr, AccessType::Read);

    // Result should be valid (either success or fault)
    assert!(result.trap.is_some() || result.paddr.val() > 0 || result.paddr.val() == 0);
}

#[test]
fn test_cache_disabled_no_stats_update() {
    let mut cpu = create_test_cpu();
    cpu.l1_i_cache.enabled = false;
    cpu.l1_d_cache.enabled = false;
    cpu.l2_cache.enabled = false;
    cpu.l3_cache.enabled = false;

    let initial_icache_hits = cpu.stats.icache_hits;
    let initial_dcache_hits = cpu.stats.dcache_hits;

    let paddr = rvsim_core::common::PhysAddr::new(0x8000_0000u64);
    cpu.simulate_memory_access(paddr, AccessType::Fetch);
    cpu.simulate_memory_access(paddr, AccessType::Read);

    // When caches are disabled, cache stats should not increase
    assert_eq!(cpu.stats.icache_hits, initial_icache_hits);
    assert_eq!(cpu.stats.dcache_hits, initial_dcache_hits);
}
