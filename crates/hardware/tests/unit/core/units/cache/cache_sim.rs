//! Cache Simulator (CacheSim) Unit Tests.
//!
//! Verifies the set-associative cache simulator with configurable replacement
//! policies and prefetchers. Tests exercise hit/miss logic, write-back penalties,
//! flushing, and disabled-cache behavior.
//!
//! The CacheSim is constructed directly from CacheConfig — no full CPU needed.
//!
//! Reference: Phase 3 — Memory Subsystem Verification.

use riscv_core::config::{
    CacheConfig, Prefetcher as PrefetcherType, ReplacementPolicy as PolicyType,
};
use riscv_core::core::units::cache::CacheSim;

// ──────────────────────────────────────────────────────────
// Helper: build a simple test cache
// ──────────────────────────────────────────────────────────

/// Creates a small, deterministic test cache.
///
/// Default: 256 bytes, 64-byte lines, 2-way set-associative, LRU,
/// no prefetcher, enabled, 1-cycle latency.
///
/// With these parameters:
///   - num_lines = 256 / 64 = 4
///   - num_sets  = 4 / 2 = 2
///   - line_bytes = 64
///
/// Set index = (addr / 64) % 2
/// Tag       = addr / (64 * 2) = addr / 128
fn test_config() -> CacheConfig {
    CacheConfig {
        enabled: true,
        size_bytes: 256,
        line_bytes: 64,
        ways: 2,
        policy: PolicyType::Lru,
        latency: 1,
        prefetcher: PrefetcherType::None,
        prefetch_table_size: 64,
        prefetch_degree: 1,
    }
}

/// Next-level (e.g., L2/DRAM) latency for miss penalty calculations.
const NEXT_LEVEL_LATENCY: u64 = 10;

// ══════════════════════════════════════════════════════════
// 1. Cold Miss
// ══════════════════════════════════════════════════════════

/// First access to any address is a compulsory (cold) miss.
/// Returns (false, 0) because install_line has no dirty victim to write back.
#[test]
fn cold_miss_returns_miss_no_penalty() {
    let mut cache = CacheSim::new(&test_config());
    let (hit, penalty) = cache.access(0x1000, false, NEXT_LEVEL_LATENCY);

    assert!(!hit, "First access should be a miss");
    assert_eq!(penalty, 0, "No dirty victim to write back on cold miss");
}

// ══════════════════════════════════════════════════════════
// 2. Warm Hit
// ══════════════════════════════════════════════════════════

/// Second access to the same address should be a hit with 0 penalty.
#[test]
fn warm_hit_returns_hit_zero_penalty() {
    let mut cache = CacheSim::new(&test_config());

    // Cold miss to install line.
    cache.access(0x1000, false, NEXT_LEVEL_LATENCY);

    // Warm hit.
    let (hit, penalty) = cache.access(0x1000, false, NEXT_LEVEL_LATENCY);
    assert!(hit, "Second access should hit");
    assert_eq!(penalty, 0, "Hits incur 0 penalty cycles");
}

/// Access to a different offset within the same cache line should hit.
#[test]
fn same_line_different_offset_hits() {
    let mut cache = CacheSim::new(&test_config());

    // Access byte 0 of a line.
    cache.access(0x1000, false, NEXT_LEVEL_LATENCY);

    // Access byte 32 of the same 64-byte line.
    let (hit, _) = cache.access(0x1000 + 32, false, NEXT_LEVEL_LATENCY);
    assert!(hit, "Different offset in same line should hit");
}

// ══════════════════════════════════════════════════════════
// 3. Set Conflict / Capacity Eviction
// ══════════════════════════════════════════════════════════

/// Fill both ways of a set, then access a third address mapping to the same set.
/// The third access should miss (evicting the LRU line).
#[test]
fn set_conflict_eviction() {
    let mut cache = CacheSim::new(&test_config());

    // Config: 2 sets, 2 ways, line_bytes=64.
    // Set index = (addr / 64) % 2
    // Tag       = addr / 128

    // Three addresses that all map to set 0:
    // addr=0:   set = (0/64) % 2 = 0,   tag = 0/128 = 0
    // addr=128: set = (128/64) % 2 = 0,  tag = 128/128 = 1
    // addr=256: set = (256/64) % 2 = 0,  tag = 256/128 = 2

    let addr_a = 0u64;
    let addr_b = 128u64;
    let addr_c = 256u64;

    // Fill both ways in set 0.
    cache.access(addr_a, false, NEXT_LEVEL_LATENCY); // miss, install in way 0
    cache.access(addr_b, false, NEXT_LEVEL_LATENCY); // miss, install in way 1

    // Verify both are present.
    assert!(cache.contains(addr_a));
    assert!(cache.contains(addr_b));

    // Third address to the same set → evicts LRU (addr_a, since addr_b was more recent).
    let (hit, _) = cache.access(addr_c, false, NEXT_LEVEL_LATENCY);
    assert!(!hit, "Third conflicting address should miss");

    // addr_a should have been evicted.
    assert!(!cache.contains(addr_a), "LRU victim should be evicted");
    assert!(
        cache.contains(addr_b),
        "Recently used address should survive"
    );
    assert!(
        cache.contains(addr_c),
        "Newly installed address should be present"
    );
}

// ══════════════════════════════════════════════════════════
// 4. Dirty Write-back Penalty
// ══════════════════════════════════════════════════════════

/// Write to a line, then evict it. The eviction should incur a write-back
/// penalty equal to next_level_latency.
#[test]
fn dirty_writeback_penalty_on_eviction() {
    let mut cache = CacheSim::new(&test_config());

    // Write to addr_a → installs dirty line in set 0, way 0.
    cache.access(0, true, NEXT_LEVEL_LATENCY);

    // Read addr_b → installs clean line in set 0, way 1.
    cache.access(128, false, NEXT_LEVEL_LATENCY);

    // Read addr_c → evicts LRU (addr_a, which is dirty).
    // Penalty should include next_level_latency for write-back.
    let (hit, penalty) = cache.access(256, false, NEXT_LEVEL_LATENCY);
    assert!(!hit);
    assert_eq!(
        penalty, NEXT_LEVEL_LATENCY,
        "Evicting dirty line should incur write-back penalty"
    );
}

/// Write to a line, access it again (hit), then evict.
/// The write-back penalty should still occur.
#[test]
fn dirty_bit_persists_across_reads() {
    let mut cache = CacheSim::new(&test_config());

    // Write addr_a (dirty).
    cache.access(0, true, NEXT_LEVEL_LATENCY);
    // Read addr_a (hit — dirty bit stays).
    cache.access(0, false, NEXT_LEVEL_LATENCY);
    // Fill way 1 in the same set.
    cache.access(128, false, NEXT_LEVEL_LATENCY);
    // Evict addr_a (dirty).
    let (_, penalty) = cache.access(256, false, NEXT_LEVEL_LATENCY);
    assert_eq!(penalty, NEXT_LEVEL_LATENCY);
}

// ══════════════════════════════════════════════════════════
// 5. Clean Eviction
// ══════════════════════════════════════════════════════════

/// Evicting a clean (read-only) line incurs no write-back penalty.
#[test]
fn clean_eviction_no_penalty() {
    let mut cache = CacheSim::new(&test_config());

    // Read addr_a (clean).
    cache.access(0, false, NEXT_LEVEL_LATENCY);
    // Read addr_b (clean).
    cache.access(128, false, NEXT_LEVEL_LATENCY);
    // Evict addr_a (clean) → no write-back.
    let (hit, penalty) = cache.access(256, false, NEXT_LEVEL_LATENCY);
    assert!(!hit);
    assert_eq!(penalty, 0, "Evicting clean line should have 0 penalty");
}

// ══════════════════════════════════════════════════════════
// 6. Flush
// ══════════════════════════════════════════════════════════

/// After flushing, previously cached dirty lines become misses.
#[test]
fn flush_invalidates_dirty_lines() {
    let mut cache = CacheSim::new(&test_config());

    // Write to install a dirty line.
    cache.access(0x1000, true, NEXT_LEVEL_LATENCY);
    assert!(cache.contains(0x1000));

    cache.flush();

    // Should miss after flush.
    assert!(!cache.contains(0x1000));
    let (hit, _) = cache.access(0x1000, false, NEXT_LEVEL_LATENCY);
    assert!(!hit, "Should miss after flush");
}

/// Flush only invalidates dirty lines; clean lines survive.
#[test]
fn flush_preserves_clean_lines() {
    let mut cache = CacheSim::new(&test_config());

    // Read (clean).
    cache.access(0x1000, false, NEXT_LEVEL_LATENCY);
    assert!(cache.contains(0x1000));

    cache.flush();

    // Clean lines should still be present (flush only clears dirty lines).
    assert!(cache.contains(0x1000), "Clean lines should survive flush");
}

// ══════════════════════════════════════════════════════════
// 7. Disabled Cache
// ══════════════════════════════════════════════════════════

/// When cache is disabled, access always returns (false, 0) — no miss penalty,
/// no hit tracking.
#[test]
fn disabled_cache_always_returns_false_zero() {
    let mut config = test_config();
    config.enabled = false;
    let mut cache = CacheSim::new(&config);

    let (hit, penalty) = cache.access(0x1000, false, NEXT_LEVEL_LATENCY);
    assert!(!hit);
    assert_eq!(penalty, 0);

    // Second access: still (false, 0).
    let (hit, penalty) = cache.access(0x1000, false, NEXT_LEVEL_LATENCY);
    assert!(!hit);
    assert_eq!(penalty, 0);
}

/// Disabled cache contains nothing.
#[test]
fn disabled_cache_contains_nothing() {
    let mut config = test_config();
    config.enabled = false;
    let mut cache = CacheSim::new(&config);

    cache.access(0x1000, false, NEXT_LEVEL_LATENCY);
    assert!(!cache.contains(0x1000));
}

// ══════════════════════════════════════════════════════════
// 8. Contains
// ══════════════════════════════════════════════════════════

/// `contains` mirrors the hit/miss status of the cache.
#[test]
fn contains_mirrors_hit_status() {
    let mut cache = CacheSim::new(&test_config());

    assert!(!cache.contains(0x2000), "Should not contain before access");

    cache.access(0x2000, false, NEXT_LEVEL_LATENCY);
    assert!(cache.contains(0x2000), "Should contain after access");

    // Evict by filling the set (2 ways).
    // addr 0x2000: set = (0x2000/64) % 2 = 0, tag = 0x2000/128 = 64
    // Need two more addresses mapping to the same set with different tags.
    // set 0: addr where (addr/64) % 2 == 0 → addr/64 is even.
    // 0x2000/64 = 128 (even) → set 0.
    // 0x2000 + 128 = 0x2080 → (0x2080/64) = 130 (even) → set 0, tag = 0x2080/128 = 65.
    cache.access(0x2000 + 128, false, NEXT_LEVEL_LATENCY);
    // 0x2000 + 256 = 0x2100 → (0x2100/64) = 132 (even) → set 0, tag = 66.
    cache.access(0x2000 + 256, false, NEXT_LEVEL_LATENCY);

    assert!(!cache.contains(0x2000), "Should not contain after eviction");
}

// ══════════════════════════════════════════════════════════
// 9. Different Line Sizes
// ══════════════════════════════════════════════════════════

/// With 32-byte lines, offsets within 32 bytes should share a line.
#[test]
fn different_line_size_32b() {
    let config = CacheConfig {
        enabled: true,
        size_bytes: 256,
        line_bytes: 32,
        ways: 2,
        policy: PolicyType::Lru,
        latency: 1,
        prefetcher: PrefetcherType::None,
        prefetch_table_size: 64,
        prefetch_degree: 1,
    };
    // num_lines = 256/32 = 8, num_sets = 8/2 = 4, line_bytes = 32.
    let mut cache = CacheSim::new(&config);

    cache.access(0x100, false, NEXT_LEVEL_LATENCY);
    // 0x100 + 16 is in the same 32-byte line → should hit.
    let (hit, _) = cache.access(0x100 + 16, false, NEXT_LEVEL_LATENCY);
    assert!(hit, "Same 32-byte line should hit");

    // 0x100 + 32 is in the NEXT line → should miss.
    let (hit, _) = cache.access(0x100 + 32, false, NEXT_LEVEL_LATENCY);
    assert!(!hit, "Next 32-byte line should miss");
}

/// With 128-byte lines, a wider range of offsets share a line.
#[test]
fn different_line_size_128b() {
    let config = CacheConfig {
        enabled: true,
        size_bytes: 1024,
        line_bytes: 128,
        ways: 2,
        policy: PolicyType::Lru,
        latency: 1,
        prefetcher: PrefetcherType::None,
        prefetch_table_size: 64,
        prefetch_degree: 1,
    };
    // num_lines = 1024/128 = 8, num_sets = 8/2 = 4, line_bytes = 128.
    let mut cache = CacheSim::new(&config);

    cache.access(0x200, false, NEXT_LEVEL_LATENCY);
    // 0x200 + 100 is within the same 128-byte line → hit.
    let (hit, _) = cache.access(0x200 + 100, false, NEXT_LEVEL_LATENCY);
    assert!(hit, "Same 128-byte line should hit");

    // 0x200 + 128 is in a different line → miss.
    let (hit, _) = cache.access(0x200 + 128, false, NEXT_LEVEL_LATENCY);
    assert!(!hit, "Different 128-byte line should miss");
}
