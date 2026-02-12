//! Tagged Prefetcher Tests.
//!
//! Verifies that the tagged prefetcher:
//! - Prefetches on demand misses (always).
//! - Does NOT prefetch on standard hits (to reduce pollution).
//! - Extends the stream when a hit occurs to a previously prefetched line.
//!
//! Reference: Phase 3 — Memory Subsystem Verification.

use riscv_core::core::units::prefetch::Prefetcher;
use riscv_core::core::units::prefetch::TaggedPrefetcher;

// ══════════════════════════════════════════════════════════
// 1. Miss triggers prefetch
// ══════════════════════════════════════════════════════════

/// A cache miss triggers prefetching of the next line(s).
#[test]
fn miss_triggers_prefetch() {
    let mut pf = TaggedPrefetcher::new(64, 1);
    let addrs = pf.observe(0x1000, false); // miss
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0], 0x1040, "Next 64-byte line after 0x1000");
}

/// Miss with degree=2 produces two sequential prefetches.
#[test]
fn miss_degree_2() {
    let mut pf = TaggedPrefetcher::new(64, 2);
    let addrs = pf.observe(0x1000, false);
    assert_eq!(addrs.len(), 2);
    assert_eq!(addrs[0], 0x1040);
    assert_eq!(addrs[1], 0x1080);
}

// ══════════════════════════════════════════════════════════
// 2. Standard hit — no prefetch
// ══════════════════════════════════════════════════════════

/// A cache hit to a line that was NOT prefetched should stay idle.
#[test]
fn standard_hit_no_prefetch() {
    let mut pf = TaggedPrefetcher::new(64, 1);
    // Hit on a line we never prefetched — filter should not match.
    let addrs = pf.observe(0x2000, true);
    assert!(addrs.is_empty(), "Standard hit should not trigger prefetch");
}

// ══════════════════════════════════════════════════════════
// 3. Hit to prefetched line extends stream
// ══════════════════════════════════════════════════════════

/// When we access a line that was previously prefetched, the prefetcher
/// should extend the stream by prefetching the next line(s).
#[test]
fn hit_to_prefetched_line_extends_stream() {
    let mut pf = TaggedPrefetcher::new(64, 1);

    // Miss at 0x1000 → prefetches 0x1040, marks it in filter.
    let pf1 = pf.observe(0x1000, false);
    assert_eq!(pf1[0], 0x1040);

    // Now simulate a hit on the prefetched address 0x1040.
    // The filter should recognize this as a prefetched line.
    let pf2 = pf.observe(0x1040, true);
    assert_eq!(pf2.len(), 1, "Hit on prefetched line should extend stream");
    assert_eq!(pf2[0], 0x1080, "Should prefetch the line after 0x1040");
}

/// Chained prefetch hits create a streaming effect.
#[test]
fn chained_prefetch_stream() {
    let mut pf = TaggedPrefetcher::new(64, 1);

    // Miss → prefetch chain starts.
    pf.observe(0x3000, false); // prefetches 0x3040
    pf.observe(0x3040, true); // hit on prefetched → prefetches 0x3080
    let addrs = pf.observe(0x3080, true); // hit on prefetched → prefetches 0x30C0
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0], 0x30C0);
}

// ══════════════════════════════════════════════════════════
// 4. Alignment
// ══════════════════════════════════════════════════════════

/// Mid-line miss still produces aligned prefetch targets.
#[test]
fn mid_line_miss_aligns() {
    let mut pf = TaggedPrefetcher::new(64, 1);
    let addrs = pf.observe(0x1020, false); // miss at offset 32 within line
    // Aligned addr = 0x1000. Next line = 0x1040.
    assert_eq!(addrs[0], 0x1040);
}

// ══════════════════════════════════════════════════════════
// 5. Edge cases
// ══════════════════════════════════════════════════════════

/// Degree 0 is clamped to 1.
#[test]
fn degree_zero_defaults_to_one() {
    let mut pf = TaggedPrefetcher::new(64, 0);
    let addrs = pf.observe(0x1000, false);
    assert_eq!(addrs.len(), 1);
}
