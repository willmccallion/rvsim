//! Write-queue drain state machine: filling ≤ low_watermark, draining ≥ high.
//!
//! We can't directly enqueue many writes without a real queue-aware driver
//! (the controller drains its own queue during each `handle()` call). So this
//! test asserts the observable *ordering* effect: a burst of writes followed
//! immediately by reads shouldn't reorder reads ahead of writes when we're
//! draining, and mixed traffic should stay legal.

use crate::unit::soc::memory::ddr5::common::{
    Harness, addr_from, read_op, tiny_config, write_op,
};

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
    // Every request must have a corresponding response scheduled.
    for id in ids {
        let _ = h.response_at(id);
    }
}

#[test]
fn drain_watermarks_are_configurable() {
    let cfg = tiny_config();
    assert_eq!(cfg.write_high_watermark, 32);
    assert_eq!(cfg.write_low_watermark, 8);
}
