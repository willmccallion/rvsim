//! Stream Prefetcher Tests.
//!
//! Verifies detection of ascending/descending sequential streams.
//! The stream prefetcher specifically targets stride-1 (one cache line)
//! access patterns and requires confidence >= 2 before prefetching.
//!
//! Reference: Phase 3 — Memory Subsystem Verification.

use riscv_core::core::units::prefetch::Prefetcher;
use riscv_core::core::units::prefetch::StreamPrefetcher;

const LINE: u64 = 64;

// ══════════════════════════════════════════════════════════
// 1. Cold start
// ══════════════════════════════════════════════════════════

/// First access never prefetches.
#[test]
fn no_prefetch_on_first_access() {
    let mut pf = StreamPrefetcher::new(64, 1);
    let addrs = pf.observe(0x1000, false);
    assert!(addrs.is_empty());
}

/// Two consecutive ascending accesses build confidence to 1, not yet 2.
#[test]
fn no_prefetch_at_confidence_1() {
    let mut pf = StreamPrefetcher::new(64, 1);
    pf.observe(0x1000, false);
    let addrs = pf.observe(0x1000 + LINE, false);
    // Direction detected (ascending), confidence = 1. Need >= 2.
    assert!(addrs.is_empty(), "Confidence 1 is not enough");
}

// ══════════════════════════════════════════════════════════
// 2. Ascending stream
// ══════════════════════════════════════════════════════════

/// Three consecutive ascending accesses → confidence reaches 2 → prefetch.
#[test]
fn ascending_stream_triggers_prefetch() {
    let mut pf = StreamPrefetcher::new(64, 1);
    pf.observe(0x1000, false); // last_addr = 0x1000
    pf.observe(0x1000 + LINE, false); // ascending, confidence = 1
    let addrs = pf.observe(0x1000 + 2 * LINE, false); // ascending, confidence = 2
    assert_eq!(addrs.len(), 1);
    // Prefetch target: aligned(0x1000 + 2*64) + 64 = 0x1080 + 0x40 = 0x10C0
    let expected = (0x1000 + 2 * LINE) + LINE;
    assert_eq!(addrs[0], expected);
}

/// Degree-2 ascending stream emits two lines ahead.
#[test]
fn ascending_degree_2() {
    let mut pf = StreamPrefetcher::new(64, 2);
    pf.observe(0x2000, false);
    pf.observe(0x2000 + LINE, false);
    let addrs = pf.observe(0x2000 + 2 * LINE, false);
    assert_eq!(addrs.len(), 2);
    assert_eq!(addrs[0], 0x2000 + 3 * LINE);
    assert_eq!(addrs[1], 0x2000 + 4 * LINE);
}

// ══════════════════════════════════════════════════════════
// 3. Descending stream
// ══════════════════════════════════════════════════════════

/// Descending sequential accesses trigger backward prefetching.
#[test]
fn descending_stream_triggers_prefetch() {
    let mut pf = StreamPrefetcher::new(64, 1);
    pf.observe(0x2000, false);
    pf.observe(0x2000 - LINE, false); // descending, confidence = 1
    let addrs = pf.observe(0x2000 - 2 * LINE, false); // confidence = 2
    assert_eq!(addrs.len(), 1);
    // Prefetch target: one line below the current access.
    let current = 0x2000 - 2 * LINE;
    let expected = current.wrapping_sub(LINE);
    assert_eq!(addrs[0], expected);
}

// ══════════════════════════════════════════════════════════
// 4. Non-sequential access resets
// ══════════════════════════════════════════════════════════

/// A non-sequential access after an ascending stream decays confidence.
#[test]
fn non_sequential_decays_confidence() {
    let mut pf = StreamPrefetcher::new(64, 1);
    pf.observe(0x1000, false);
    pf.observe(0x1000 + LINE, false); // asc, confidence = 1
    // Jump to a random address (non-sequential).
    let addrs = pf.observe(0x5000, false);
    assert!(addrs.is_empty(), "Non-sequential should not prefetch");
}

/// After a direction switch, old direction confidence must rebuild.
#[test]
fn direction_switch_resets_confidence() {
    let mut pf = StreamPrefetcher::new(64, 1);
    // Build ascending confidence.
    pf.observe(0x1000, false);
    pf.observe(0x1000 + LINE, false); // asc, conf=1
    pf.observe(0x1000 + 2 * LINE, false); // asc, conf=2 (would prefetch)

    // Switch to descending.
    let current = 0x1000 + 2 * LINE;
    pf.observe(current - LINE, false); // desc, new direction → conf=1
    let addrs = pf.observe(current - 2 * LINE, false); // desc, conf=2
    assert_eq!(
        addrs.len(),
        1,
        "Descending should prefetch after rebuilding confidence"
    );
}
