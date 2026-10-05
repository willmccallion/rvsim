//! The L1D's store-miss prefetcher: runs of store misses to adjacent lines
//! in a 4 KiB page, kept ahead in the L2.

use crate::soc::cache::prefetch::StoreStreamPrefetcher;

const LINE: u64 = 64;

fn prefetcher(l2_lines: usize) -> StoreStreamPrefetcher {
    StoreStreamPrefetcher::new(LINE as usize, 4, l2_lines)
}

#[test]
fn two_adjacent_misses_are_not_yet_a_run() {
    let mut pf = prefetcher(4);

    let first = pf.observe(0x1000);
    let second = pf.observe(0x1040);

    assert!(first.is_empty() && second.is_empty());
}

#[test]
fn a_third_miss_in_the_same_direction_sends_the_run_ahead() {
    let mut pf = prefetcher(3);
    let _ = pf.observe(0x1000);
    let _ = pf.observe(0x1040);

    let sent = pf.observe(0x1080);

    assert_eq!(sent, vec![0x10C0, 0x1100, 0x1140]);
}

#[test]
fn a_run_sends_each_line_once() {
    let mut pf = prefetcher(3);
    for addr in [0x1000, 0x1040, 0x1080] {
        let _ = pf.observe(addr);
    }

    let sent = pf.observe(0x10C0);

    assert_eq!(sent, vec![0x1180]);
}

#[test]
fn a_descending_run_prefetches_downward() {
    let mut pf = prefetcher(2);
    let _ = pf.observe(0x1F00);
    let _ = pf.observe(0x1EC0);

    let sent = pf.observe(0x1E80);

    assert_eq!(sent, vec![0x1E40, 0x1E00]);
}

#[test]
fn a_run_stops_at_its_4k_page() {
    let mut pf = prefetcher(4);
    let _ = pf.observe(0x1F40);
    let _ = pf.observe(0x1F80);

    let sent = pf.observe(0x1FC0);

    assert!(sent.is_empty(), "every line ahead is in the next page: {sent:x?}");
}

#[test]
fn scattered_misses_send_nothing() {
    let mut pf = prefetcher(4);

    let sent: Vec<u64> =
        [0x1000, 0x5000, 0x1200, 0x9040].into_iter().flat_map(|a| pf.observe(a)).collect();

    assert!(sent.is_empty());
}
