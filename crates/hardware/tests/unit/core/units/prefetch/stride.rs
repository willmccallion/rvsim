//! Stride Prefetcher Tests.
//!
//! Verifies that the stride prefetcher correctly detects constant-stride
//! access patterns, builds confidence before prefetching, and emits
//! properly aligned addresses at the correct stride.
//!
//! Reference: Phase 3 — Memory Subsystem Verification.

use riscv_core::core::units::prefetch::Prefetcher;
use riscv_core::core::units::prefetch::StridePrefetcher;

// ══════════════════════════════════════════════════════════
// 1. Cold start — no prefetching
// ══════════════════════════════════════════════════════════

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
    // First access establishes last_addr.
    pf.observe(0x1000, false);
    // Second access establishes stride, but confidence starts at 0 and
    // the implementation needs stride == entry.stride to increment.
    // Since entry.stride is initially 0 and current_stride != 0,
    // confidence stays at 0 and stride is set.
    let addrs = pf.observe(0x1100, false);
    assert!(addrs.is_empty());
}

// ══════════════════════════════════════════════════════════
// 2. Stride detection and prefetching
// ══════════════════════════════════════════════════════════

/// After enough repeated accesses with the same stride, prefetch triggers.
/// The stride prefetcher indexes by (addr >> 6) & mask, so we need accesses
/// that all hash to the same table entry for the confidence counter to build.
#[test]
fn constant_stride_triggers_prefetch() {
    let mut pf = StridePrefetcher::new(64, 64, 1);

    // Use addresses that all map to the same table index.
    // Index = (addr >> 6) & 63. For addr 0x1000: idx = (0x1000 >> 6) & 63 = 64 & 63 = 0.
    // Stride of 0x100 (256 bytes). addr 0x1100: idx = (0x1100 >> 6) & 63 = 68 & 63 = 4.
    // These map to different indices! We need same-index accesses.
    // Let's use stride = 64*64 = 4096 so that (addr >> 6) increments by 64
    // and wraps back to the same index (mod 64).

    // Actually, the simplest approach: stride must be such that all accesses
    // map to the same index. idx = (addr >> 6) & 63.
    // Access 0: addr=0x0000 → idx=0
    // For idx to stay 0, we need (addr >> 6) % 64 == 0, so addr must be
    // a multiple of 64*64 = 4096.
    let stride = 4096u64;
    let base = 0u64;

    // Trace through the stride detection state machine:
    //   Step 0: addr=0.       Stride 0 matches initial stride 0 → conf 0→1. last_addr=0.
    //   Step 1: addr=4096.    Stride 4096 != 0 → conf 1→0 (decrement). last_addr=4096.
    //   Step 2: addr=8192.    Stride 4096 != 0, conf==0 → set stride=4096. last_addr=8192.
    //   Step 3: addr=12288.   Stride 4096 matches → conf 0→1.
    //   Step 4: addr=16384.   Stride matches → conf 1→2.
    //   Step 5: addr=20480.   Stride matches → conf 2→3.
    //   Step 6: addr=24576.   Stride matches, conf==3 → PREFETCH.
    for i in 0..7 {
        pf.observe(base + stride * i, false);
    }

    // The 8th access should trigger a prefetch (confidence is already 3).
    let addrs = pf.observe(base + stride * 7, false);
    assert!(
        !addrs.is_empty(),
        "Should prefetch after confidence reaches 3"
    );

    // The prefetch target should be base + stride*8, aligned to 64 bytes.
    let expected = (base + stride * 8) & !63;
    assert_eq!(addrs[0], expected);
}

// ══════════════════════════════════════════════════════════
// 3. Stride change resets confidence
// ══════════════════════════════════════════════════════════

/// Changing the stride decrements confidence and eventually resets.
#[test]
fn stride_change_reduces_confidence() {
    let mut pf = StridePrefetcher::new(64, 64, 1);
    let stride = 4096u64;

    // Build confidence to 3 (needs 7 accesses, see constant_stride test).
    for i in 0..7 {
        pf.observe(i * stride, false);
    }
    // Confidence is 3 at this point for index 0.

    // Change stride — access with a different stride.
    // New address doesn't match the old stride, so confidence decrements.
    let off = stride * 7 + 128; // different stride from entry
    let addrs = pf.observe(off, false);
    // Confidence was 3, decremented to 2 — no prefetch because
    // the stride doesn't match.
    assert!(addrs.is_empty(), "Stride changed → no prefetch");
}

// ══════════════════════════════════════════════════════════
// 4. Degree > 1
// ══════════════════════════════════════════════════════════

/// Degree-2 prefetcher emits two stride-ahead addresses once warmed up.
#[test]
fn degree_2_emits_two_addresses() {
    let mut pf = StridePrefetcher::new(64, 64, 2);
    let stride = 4096u64;

    // 7 warmup accesses to reach confidence 3.
    for i in 0..7 {
        pf.observe(i * stride, false);
    }

    let addrs = pf.observe(7 * stride, false);
    assert_eq!(addrs.len(), 2, "Degree 2 should emit 2 prefetches");
}
