//! Stream Prefetcher Tests.
//!
//! Verifies detection of ascending/descending sequential streams.
//! The stream prefetcher specifically targets stride-1 (one cache line)
//! access patterns and requires confidence >= 2 before prefetching.
//!
//! Reference: Phase 3 — Memory Subsystem Verification.

use super::observe;
use crate::soc::cache::prefetch::Lookup;
use crate::soc::cache::prefetch::StreamPrefetcher;

const LINE: u64 = 64;

/// First access never prefetches.
#[test]
fn no_prefetch_on_first_access() {
    let mut pf = StreamPrefetcher::new(64, std::num::NonZero::new(1).unwrap());
    let addrs = observe(&mut pf, 0x1000, None, Lookup::Miss);
    assert!(addrs.is_empty());
}

/// Two consecutive ascending accesses build confidence to 1, not yet 2.
#[test]
fn no_prefetch_at_confidence_1() {
    let mut pf = StreamPrefetcher::new(64, std::num::NonZero::new(1).unwrap());
    observe(&mut pf, 0x1000, None, Lookup::Miss);
    let addrs = observe(&mut pf, 0x1000 + LINE, None, Lookup::Miss);
    // Direction detected (ascending), confidence = 1. Need >= 2.
    assert!(addrs.is_empty(), "Confidence 1 is not enough");
}

/// Three consecutive ascending accesses → confidence reaches 2 → prefetch.
#[test]
fn ascending_stream_triggers_prefetch() {
    let mut pf = StreamPrefetcher::new(64, std::num::NonZero::new(1).unwrap());
    observe(&mut pf, 0x1000, None, Lookup::Miss); // last_addr = 0x1000
    observe(&mut pf, 0x1000 + LINE, None, Lookup::Miss); // ascending, confidence = 1
    let addrs = observe(&mut pf, 0x1000 + 2 * LINE, None, Lookup::Miss); // ascending, confidence = 2
    assert_eq!(addrs.len(), 1);
    // Prefetch target: aligned(0x1000 + 2*64) + 64 = 0x1080 + 0x40 = 0x10C0
    let expected = (0x1000 + 2 * LINE) + LINE;
    assert_eq!(addrs[0], expected);
}

/// Degree-2 ascending stream emits two lines ahead.
#[test]
fn ascending_degree_2() {
    let mut pf = StreamPrefetcher::new(64, std::num::NonZero::new(2).unwrap());
    observe(&mut pf, 0x2000, None, Lookup::Miss);
    observe(&mut pf, 0x2000 + LINE, None, Lookup::Miss);
    let addrs = observe(&mut pf, 0x2000 + 2 * LINE, None, Lookup::Miss);
    assert_eq!(addrs.len(), 2);
    assert_eq!(addrs[0], 0x2000 + 3 * LINE);
    assert_eq!(addrs[1], 0x2000 + 4 * LINE);
}

/// Descending sequential accesses trigger backward prefetching.
#[test]
fn descending_stream_triggers_prefetch() {
    let mut pf = StreamPrefetcher::new(64, std::num::NonZero::new(1).unwrap());
    observe(&mut pf, 0x2000, None, Lookup::Miss);
    observe(&mut pf, 0x2000 - LINE, None, Lookup::Miss); // descending, confidence = 1
    let addrs = observe(&mut pf, 0x2000 - 2 * LINE, None, Lookup::Miss); // confidence = 2
    assert_eq!(addrs.len(), 1);
    // Prefetch target: one line below the current access.
    let current = 0x2000 - 2 * LINE;
    let expected = current.wrapping_sub(LINE);
    assert_eq!(addrs[0], expected);
}

/// A non-sequential access after an ascending stream decays confidence.
#[test]
fn non_sequential_decays_confidence() {
    let mut pf = StreamPrefetcher::new(64, std::num::NonZero::new(1).unwrap());
    observe(&mut pf, 0x1000, None, Lookup::Miss);
    observe(&mut pf, 0x1000 + LINE, None, Lookup::Miss); // asc, confidence = 1
    // Jump to a random address (non-sequential).
    let addrs = observe(&mut pf, 0x5000, None, Lookup::Miss);
    assert!(addrs.is_empty(), "Non-sequential should not prefetch");
}

/// After a direction switch, old direction confidence must rebuild.
#[test]
fn direction_switch_resets_confidence() {
    let mut pf = StreamPrefetcher::new(64, std::num::NonZero::new(1).unwrap());
    // Build ascending confidence.
    observe(&mut pf, 0x1000, None, Lookup::Miss);
    observe(&mut pf, 0x1000 + LINE, None, Lookup::Miss); // asc, conf=1
    observe(&mut pf, 0x1000 + 2 * LINE, None, Lookup::Miss); // asc, conf=2 (would prefetch)

    // Switch to descending.
    let current = 0x1000 + 2 * LINE;
    observe(&mut pf, current - LINE, None, Lookup::Miss); // desc, new direction → conf=1
    let addrs = observe(&mut pf, current - 2 * LINE, None, Lookup::Miss); // desc, conf=2
    assert_eq!(addrs.len(), 1, "Descending should prefetch after rebuilding confidence");
}
