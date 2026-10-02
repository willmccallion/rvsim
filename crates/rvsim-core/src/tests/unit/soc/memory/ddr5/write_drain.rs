//! Posted writes and the write-queue drain policy.

use crate::sim::packet::DramCmdKind;
use crate::tests::unit::soc::memory::ddr5::common::{
    Harness, addr_from, read_op, tiny_config, write_op,
};

#[test]
fn write_is_acknowledged_on_admission_before_it_reaches_dram() {
    let cfg = tiny_config();
    let mut h = Harness::new(cfg);
    let a = addr_from(&cfg, 0, 0, 0, 0, 0);
    let id = h.issue(a, 10, write_op());
    let ack = h.response_at(id);
    assert_eq!(ack, 10 + cfg.frontend_latency, "posted write acks after the front end");
    h.run_until(400);
    let writes = h.commands_of(DramCmdKind::Write);
    assert_eq!(writes.len(), 1);
    assert!(
        writes[0].fire_at > ack,
        "WR command at {} should follow the ack at {ack}",
        writes[0].fire_at
    );
}

#[test]
fn read_to_a_queued_write_is_served_from_the_write_queue() {
    let mut cfg = tiny_config();
    // Keep the write from draining by holding it below the high watermark
    // while reads are pending.
    cfg.write_high_watermark = 64;
    let mut h = Harness::new(cfg);
    let line = addr_from(&cfg, 0, 0, 0, 0, 0);
    let other = addr_from(&cfg, 0, 1, 0, 3, 0);
    let _ = h.issue(other, 0, read_op());
    let _ = h.issue(line, 0, write_op());
    let hit = h.issue(line, 1, read_op());
    let resp = h.response_at(hit);
    assert_eq!(resp, 1 + cfg.frontend_latency);
    h.run_until(400);
    let reads = h.commands_of(DramCmdKind::Read);
    assert_eq!(reads.len(), 1, "the forwarded read must not touch DRAM");
}

#[test]
fn writes_to_the_same_line_merge_into_one_dram_write() {
    let cfg = tiny_config();
    let mut h = Harness::new(cfg);
    let line = addr_from(&cfg, 0, 0, 0, 0, 0);
    let _ = h.issue(addr_from(&cfg, 0, 1, 0, 3, 0), 0, read_op());
    let w1 = h.issue(line, 0, write_op());
    let w2 = h.issue(line, 0, write_op());
    let _ = h.response_at(w1);
    let _ = h.response_at(w2);
    h.run_until(600);
    let writes = h.commands_of(DramCmdKind::Write);
    assert_eq!(writes.len(), 1, "merged writes drain as one WR");
}

#[test]
fn read_queue_capacity_delays_admission_until_a_slot_frees() {
    let mut cfg = tiny_config();
    cfg.read_queue_entries = 1;
    let mut h = Harness::new(cfg);
    let a = addr_from(&cfg, 0, 0, 0, 0, 0);
    let b = addr_from(&cfg, 0, 1, 0, 0, 0);
    let ida = h.issue(a, 0, read_op());
    let idb = h.issue(b, 0, read_op());
    let end_a = h.response_at(ida);
    let end_b = h.response_at(idb);
    let reads = h.commands_of(DramCmdKind::Read);
    // `b` can only enter the queue once `a`'s column command retired it, so
    // its ACT (and hence RD) comes after `a`'s RD.
    let acts = h.commands_of(DramCmdKind::Activate);
    assert!(
        acts[1].fire_at > reads[0].fire_at,
        "ACT for b at {} before RD for a at {}",
        acts[1].fire_at,
        reads[0].fire_at
    );
    assert!(end_b > end_a);
}

#[test]
fn mixed_traffic_serves_all_requests() {
    let cfg = tiny_config();
    let mut h = Harness::new(cfg);
    let mut ids = Vec::new();
    for i in 0..8 {
        let bg = (i & 1) as u8;
        let bank = ((i >> 1) & 1) as u8;
        let row = (i >> 2) as u32;
        let a = addr_from(&cfg, 0, bg, bank, row, 0);
        let op = if i % 2 == 0 { read_op() } else { write_op() };
        ids.push(h.issue(a, 0, op));
    }
    for id in ids {
        let _ = h.response_at(id);
    }
}

#[test]
fn drain_watermarks_follow_gem5_defaults() {
    let cfg = Ddr5ConfigDefaults::get();
    assert_eq!(cfg.write_queue_entries, 64);
    assert_eq!(cfg.write_high_watermark, 54);
    assert_eq!(cfg.write_low_watermark, 32);
    assert_eq!(cfg.min_writes_per_switch, 16);
}

struct Ddr5ConfigDefaults;
impl Ddr5ConfigDefaults {
    fn get() -> crate::config::ddr5::Ddr5Config {
        crate::config::ddr5::Ddr5Config::default()
    }
}
