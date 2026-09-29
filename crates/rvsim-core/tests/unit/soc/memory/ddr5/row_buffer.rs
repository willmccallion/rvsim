//! Row-hit and row-miss timing on a single bank.

use crate::unit::soc::memory::ddr5::common::{
    Harness, addr_from, controller_latency, read_op, tiny_config,
};
use rvsim_core::config::ddr5::Ddr5Timing;

#[test]
fn cold_read_pays_act_plus_rcd_plus_cas_plus_burst() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let addr = addr_from(&cfg, 0, 0, 0, 0, 0);
    let id = h.issue(addr, 0, read_op());
    let resp = h.response_at(id);
    // ACT at 0 (cmd bus free), RD at t_rcd, data at t_rcd + t_cas,
    // burst end at + bl_half, then the controller's fixed latency.
    assert_eq!(resp, t.t_rcd + t.t_cas + t.bl_half + controller_latency(&cfg));
}

#[test]
fn row_hit_second_read_only_pays_cas_plus_burst() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let a1 = addr_from(&cfg, 0, 0, 0, 0, 0);
    let a2 = addr_from(&cfg, 0, 0, 0, 0, 1);
    let first = h.issue(a1, 0, read_op());
    let _ = h.response_at(first);
    let second = h.issue(a2, 200, read_op());
    let resp = h.response_at(second);
    // Row already open. The controller issues the column command at 200,
    // well past the first read's data end.
    assert_eq!(resp, 200 + t.t_cas + t.bl_half + controller_latency(&cfg));
}

#[test]
fn same_bank_row_miss_pays_pre_plus_act_plus_col() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let a1 = addr_from(&cfg, 0, 0, 0, 0, 0);
    let a2 = addr_from(&cfg, 0, 0, 0, 1, 0);
    let first = h.issue(a1, 0, read_op());
    let _ = h.response_at(first);
    // Issue the row-miss well after the first burst so command-bus /
    // data-bus availability is not the binding constraint.
    let start = 500;
    let id = h.issue(a2, start, read_op());
    let resp = h.response_at(id);
    // From `start`: PRE (tRAS from the ACT at cycle 0 has long elapsed),
    // tRP to ACT, tRCD to column, tCAS to data start, bl_half to data end.
    let expected = start + t.t_rp + t.t_rcd + t.t_cas + t.bl_half + controller_latency(&cfg);
    assert_eq!(resp, expected);
}

#[test]
fn back_to_back_same_bank_group_reads_honor_ccd_l() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    // Same bank group, different banks, both queued before any command:
    // the second column command is at least tCCD_L after the first.
    let a1 = addr_from(&cfg, 0, 0, 0, 0, 0);
    let a2 = addr_from(&cfg, 0, 0, 1, 0, 0);
    let id1 = h.issue(a1, 0, read_op());
    let id2 = h.issue(a2, 0, read_op());
    let _ = h.response_at(id1);
    let _ = h.response_at(id2);
    let reads = h.commands_of(rvsim_core::sim::packet::DramCmdKind::Read);
    assert_eq!(reads.len(), 2);
    assert!(
        reads[1].fire_at >= reads[0].fire_at + t.t_ccd_l,
        "RD spacing {} < tCCD_L {}",
        reads[1].fire_at - reads[0].fire_at,
        t.t_ccd_l
    );
}
