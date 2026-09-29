//! Stride Prefetcher Tests.
//!
//! Verifies that the stride prefetcher correctly detects constant-stride
//! access patterns, builds confidence before prefetching, and emits
//! properly aligned addresses at the correct stride.
//!
//! Reference: Phase 3 — Memory Subsystem Verification.

use rvsim_core::soc::cache::prefetch::Prefetcher;
use rvsim_core::soc::cache::prefetch::StridePrefetcher;

/// First access never triggers a prefetch (no history).
#[test]
fn no_prefetch_on_first_access() {
    let mut pf = StridePrefetcher::new(64, 64, 1);
    let addrs = pf.observe(0x1000, false);
    assert!(addrs.is_empty(), "No history yet → no prefetch");
}

/// Two accesses with same stride are not enough — confidence must build.
#[test]
fn no_prefetch_at_low_confidence() {
    let mut pf = StridePrefetcher::new(64, 64, 1);
    pf.observe(0x1000, false);
    let addrs = pf.observe(0x1100, false);
    assert!(addrs.is_empty());
}

/// After enough repeated accesses with the same stride, prefetch triggers.
/// The stride prefetcher indexes by (addr >> 6) & mask, so we need accesses
/// that all hash to the same table entry for the confidence counter to build.
#[test]
fn constant_stride_triggers_prefetch() {
    let mut pf = StridePrefetcher::new(64, 64, 1);

    // Stride 4096 keeps every access on table index 0 (idx = (addr>>6) & 63).
    // 7 accesses brings confidence to 3, after which the next access prefetches.
    let stride = 4096u64;
    let base = 0u64;

    for i in 0..7 {
        pf.observe(base + stride * i, false);
    }

    // The 8th access should trigger a prefetch (confidence is already 3).
    let addrs = pf.observe(base + stride * 7, false);
    assert!(!addrs.is_empty(), "Should prefetch after confidence reaches 3");

    // The prefetch target should be base + stride*8, aligned to 64 bytes.
    let expected = (base + stride * 8) & !63;
    assert_eq!(addrs[0], expected);
}

/// Changing the stride decrements confidence and eventually resets.
#[test]
fn stride_change_reduces_confidence() {
    let mut pf = StridePrefetcher::new(64, 64, 1);
    let stride = 4096u64;

    for i in 0..7 {
        pf.observe(i * stride, false);
    }

    let off = stride * 7 + 128; // different stride from entry
    let addrs = pf.observe(off, false);
    assert!(addrs.is_empty(), "Stride changed → no prefetch");
}

/// Degree-2 prefetcher emits two stride-ahead addresses once warmed up.
#[test]
fn degree_2_emits_two_addresses() {
    let mut pf = StridePrefetcher::new(64, 64, 2);
    let stride = 4096u64;

    for i in 0..7 {
        pf.observe(i * stride, false);
    }

    let addrs = pf.observe(7 * stride, false);
    assert_eq!(addrs.len(), 2, "Degree 2 should emit 2 prefetches");
}
