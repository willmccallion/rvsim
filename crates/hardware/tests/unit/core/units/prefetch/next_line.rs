//! Next-Line Prefetcher Tests.
//!
//! Verifies that the next-line prefetcher emits the correct number of
//! sequential, line-aligned prefetch addresses on every access.
//!
//! Reference: Phase 3 — Memory Subsystem Verification.

use riscv_core::core::units::prefetch::NextLinePrefetcher;
use riscv_core::core::units::prefetch::Prefetcher;

// ══════════════════════════════════════════════════════════
// 1. Basic operation
// ══════════════════════════════════════════════════════════

/// Degree-1 prefetcher emits exactly one address — the next cache line.
#[test]
fn degree_1_emits_one_next_line() {
    let mut pf = NextLinePrefetcher::new(64, 1);
    let addrs = pf.observe(0x1000, false);
    assert_eq!(addrs.len(), 1);
    assert_eq!(addrs[0], 0x1040, "Next 64-byte line after 0x1000");
}

/// Degree-2 prefetcher emits two sequential lines.
#[test]
fn degree_2_emits_two_lines() {
    let mut pf = NextLinePrefetcher::new(64, 2);
    let addrs = pf.observe(0x1000, false);
    assert_eq!(addrs.len(), 2);
    assert_eq!(addrs[0], 0x1040);
    assert_eq!(addrs[1], 0x1080);
}

/// Degree-4 prefetcher emits four sequential lines.
#[test]
fn degree_4_emits_four_lines() {
    let mut pf = NextLinePrefetcher::new(64, 4);
    let addrs = pf.observe(0x2000, true);
    assert_eq!(addrs.len(), 4);
    for (k, addr) in addrs.iter().enumerate() {
        assert_eq!(*addr, 0x2000 + 64 * (k as u64 + 1));
    }
}

// ══════════════════════════════════════════════════════════
// 2. Alignment
// ══════════════════════════════════════════════════════════

/// Access within a line is aligned down before computing next line.
#[test]
fn mid_line_access_aligns_down() {
    let mut pf = NextLinePrefetcher::new(64, 1);
    // 0x1020 is at offset 32 within the 64-byte line starting at 0x1000.
    let addrs = pf.observe(0x1020, false);
    assert_eq!(
        addrs[0], 0x1040,
        "Prefetch target should be aligned next line"
    );
}

/// Non-power-of-two offset within a line still aligns correctly.
#[test]
fn odd_offset_aligns() {
    let mut pf = NextLinePrefetcher::new(64, 1);
    let addrs = pf.observe(0x1037, false);
    assert_eq!(addrs[0], 0x1040);
}

// ══════════════════════════════════════════════════════════
// 3. Hit/Miss independence
// ══════════════════════════════════════════════════════════

/// NextLine prefetcher does NOT differentiate between hits and misses.
#[test]
fn hit_and_miss_produce_same_result() {
    let mut pf_hit = NextLinePrefetcher::new(64, 1);
    let mut pf_miss = NextLinePrefetcher::new(64, 1);
    let addr = 0x3000;

    let on_hit = pf_hit.observe(addr, true);
    let on_miss = pf_miss.observe(addr, false);
    assert_eq!(on_hit, on_miss);
}

// ══════════════════════════════════════════════════════════
// 4. Different line sizes
// ══════════════════════════════════════════════════════════

/// 32-byte line size produces 32-byte-spaced prefetches.
#[test]
fn line_size_32() {
    let mut pf = NextLinePrefetcher::new(32, 1);
    let addrs = pf.observe(0x1000, false);
    assert_eq!(addrs[0], 0x1020);
}

/// 128-byte line size produces 128-byte-spaced prefetches.
#[test]
fn line_size_128() {
    let mut pf = NextLinePrefetcher::new(128, 2);
    let addrs = pf.observe(0x1000, false);
    assert_eq!(addrs[0], 0x1080);
    assert_eq!(addrs[1], 0x1100);
}

// ══════════════════════════════════════════════════════════
// 5. Edge cases
// ══════════════════════════════════════════════════════════

/// Degree 0 is clamped to 1 (implementation safeguard).
#[test]
fn degree_zero_defaults_to_one() {
    let mut pf = NextLinePrefetcher::new(64, 0);
    let addrs = pf.observe(0x1000, false);
    assert_eq!(addrs.len(), 1);
}
