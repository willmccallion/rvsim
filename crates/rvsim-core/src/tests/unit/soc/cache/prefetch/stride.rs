//! Stride Prefetcher Tests.
//!
//! Verifies that the stride prefetcher learns a stride per load PC, builds
//! confidence before prefetching, and emits line-aligned addresses
//! `degree` strides ahead.

use crate::common::VirtAddr;
use crate::soc::cache::prefetch::Prefetcher;
use crate::soc::cache::prefetch::StridePrefetcher;

const LOAD: Option<VirtAddr> = Some(VirtAddr(0x8000_1000));
const OTHER_LOAD: Option<VirtAddr> = Some(VirtAddr(0x8000_1004));

/// Accesses a load makes before its stride has saturated confidence: one
/// to allocate the entry, one to set the stride, three to confirm it.
const WARMUP: u64 = 5;

/// Feeds `count` accesses `stride` bytes apart from `base` by the load at
/// `pc`, and returns what the last one prefetched.
fn stream(
    pf: &mut StridePrefetcher,
    pc: Option<VirtAddr>,
    base: u64,
    stride: i64,
    count: u64,
) -> Vec<u64> {
    let mut last = Vec::new();
    for i in 0..count {
        last = pf.observe(base.wrapping_add_signed(stride * i as i64), pc, false);
    }
    last
}

#[test]
fn a_loads_first_access_prefetches_nothing() {
    let mut pf = StridePrefetcher::new(64, 64, 1);

    let prefetches = pf.observe(0x1000, LOAD, false);

    assert!(prefetches.is_empty());
}

#[test]
fn a_stride_prefetches_only_once_confidence_saturates() {
    let mut pf = StridePrefetcher::new(64, 64, 1);

    let warming = stream(&mut pf, LOAD, 0x1_0000, 256, WARMUP);
    let trained = pf.observe(0x1_0000 + 256 * WARMUP, LOAD, false);

    assert!(warming.is_empty());
    assert_eq!(trained, vec![0x1_0000 + 256 * (WARMUP + 1)]);
}

#[test]
fn a_256_byte_stride_prefetches_degree_strides_ahead() {
    let mut pf = StridePrefetcher::new(64, 64, 4);
    let base = 0x2_0000;

    let prefetches = stream(&mut pf, LOAD, base, 256, WARMUP + 1);

    let last = base + 256 * WARMUP;
    assert_eq!(prefetches, vec![last + 256, last + 512, last + 768, last + 1024]);
}

#[test]
fn a_stride_shorter_than_a_line_prefetches_the_following_lines() {
    let mut pf = StridePrefetcher::new(64, 64, 2);
    let base = 0x2_1000;

    let prefetches = stream(&mut pf, LOAD, base, 8, WARMUP + 1);

    let line = (base + 8 * WARMUP) & !63;
    assert_eq!(prefetches, vec![line + 64, line + 128]);
}

#[test]
fn a_negative_stride_prefetches_downward() {
    let mut pf = StridePrefetcher::new(64, 64, 1);
    let base = 0x3_0000;

    let prefetches = stream(&mut pf, LOAD, base, -192, WARMUP + 1);

    assert_eq!(prefetches, vec![base - 192 * (WARMUP + 1)]);
}

#[test]
fn prefetch_targets_are_line_aligned() {
    let mut pf = StridePrefetcher::new(64, 64, 1);

    let prefetches = stream(&mut pf, LOAD, 0x4_0008, 200, WARMUP + 1);

    assert_eq!(prefetches, vec![(0x4_0008 + 200 * (WARMUP + 1)) & !63]);
}

#[test]
fn interleaved_loads_each_learn_their_own_stride() {
    let mut pf = StridePrefetcher::new(64, 64, 1);
    let (a, b) = (0x5_0000u64, 0x9_0000u64);
    let (mut last_a, mut last_b) = (Vec::new(), Vec::new());

    for i in 0..=WARMUP {
        last_a = pf.observe(a + 256 * i, LOAD, false);
        last_b = pf.observe(b + 4096 * i, OTHER_LOAD, false);
    }

    assert_eq!(last_a, vec![a + 256 * (WARMUP + 1)]);
    assert_eq!(last_b, vec![b + 4096 * (WARMUP + 1)]);
}

#[test]
fn accesses_without_a_pc_do_not_train() {
    let mut pf = StridePrefetcher::new(64, 64, 1);

    let prefetches = stream(&mut pf, None, 0x6_0000, 256, 3 * WARMUP);

    assert!(prefetches.is_empty());
}

#[test]
fn a_changed_stride_does_not_prefetch() {
    let mut pf = StridePrefetcher::new(64, 64, 1);
    let base = 0x7_0000;
    let _ = stream(&mut pf, LOAD, base, 256, WARMUP + 1);

    let prefetches = pf.observe(base + 256 * WARMUP + 1000, LOAD, false);

    assert!(prefetches.is_empty());
}

#[test]
fn a_repeated_address_prefetches_nothing() {
    let mut pf = StridePrefetcher::new(64, 64, 1);

    let prefetches = stream(&mut pf, LOAD, 0x8_0000, 0, 3 * WARMUP);

    assert!(prefetches.is_empty());
}

#[test]
fn a_load_that_takes_over_an_entry_starts_untrained() {
    let mut pf = StridePrefetcher::new(64, 64, 1);
    let alias = LOAD.map(|pc| VirtAddr(pc.val() + 64 * 2));
    let _ = stream(&mut pf, LOAD, 0x9_0000, 256, WARMUP + 1);
    let _ = pf.observe(0xA_0000, alias, false);

    let prefetches = pf.observe(0x9_0000 + 256 * (WARMUP + 1), LOAD, false);

    assert!(prefetches.is_empty());
}
