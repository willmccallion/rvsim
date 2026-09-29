//! Request selection: FR-FCFS serves an open-row hit ahead of an older row
//! miss; FCFS keeps arrival order.

use crate::unit::soc::memory::ddr5::common::{Harness, addr_from, read_op, tiny_config};
use rvsim_core::config::ddr5::SchedulerKind;
use rvsim_core::sim::packet::DramCmdKind;
use rvsim_core::soc::memory::ddr5::{Candidate, Fcfs, FrFcfs, MemScheduler};

/// Opens row 0 of bank 0, then queues a row-1 miss followed by a row-0 hit
/// to the same bank in the same cycle. Returns the rows of the READ
/// commands in issue order.
fn read_rows_after_miss_then_hit(kind: SchedulerKind) -> Vec<u32> {
    let mut cfg = tiny_config();
    cfg.scheduler = kind;
    let mut h = Harness::new(cfg);
    let opener = h.issue(addr_from(&cfg, 0, 0, 0, 0, 0), 0, read_op());
    let _ = h.response_at(opener);
    let miss = h.issue(addr_from(&cfg, 0, 0, 0, 1, 0), 300, read_op());
    let hit = h.issue(addr_from(&cfg, 0, 0, 0, 0, 1), 300, read_op());
    let _ = h.response_at(miss);
    let _ = h.response_at(hit);
    h.commands_of(DramCmdKind::Read).iter().map(|c| c.row).collect()
}

#[test]
fn fr_fcfs_serves_the_row_hit_before_the_older_row_miss() {
    assert_eq!(read_rows_after_miss_then_hit(SchedulerKind::FrFcfs), vec![0, 0, 1]);
}

#[test]
fn fcfs_serves_requests_in_arrival_order() {
    assert_eq!(read_rows_after_miss_then_hit(SchedulerKind::Fcfs), vec![0, 1, 0]);
}

#[test]
fn fr_fcfs_prefers_a_seamless_hit_over_anything_older() {
    let candidates = [
        Candidate { row_hit: false, ready_at: 5 },
        Candidate { row_hit: true, ready_at: 50 },
        Candidate { row_hit: true, ready_at: 10 },
    ];
    assert_eq!(FrFcfs.pick(&candidates, 10), Some(2));
}

#[test]
fn fr_fcfs_falls_back_to_the_earliest_ready_request_preferring_hits() {
    let candidates = [
        Candidate { row_hit: false, ready_at: 40 },
        Candidate { row_hit: false, ready_at: 30 },
        Candidate { row_hit: true, ready_at: 30 },
    ];
    assert_eq!(FrFcfs.pick(&candidates, 10), Some(2));
    assert_eq!(FrFcfs.pick(&candidates[..2], 10), Some(1));
    assert_eq!(FrFcfs.pick(&[], 10), None);
}

#[test]
fn fcfs_always_takes_the_head_of_the_queue() {
    let candidates =
        [Candidate { row_hit: false, ready_at: 90 }, Candidate { row_hit: true, ready_at: 0 }];
    assert_eq!(Fcfs.pick(&candidates, 10), Some(0));
    assert_eq!(Fcfs.pick(&[], 10), None);
}
