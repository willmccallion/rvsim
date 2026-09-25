//! Row-hit and row-miss timing on a single bank.

use crate::unit::soc::memory::ddr5::common::{Harness, addr_from, read_op, tiny_config};
use rvsim_core::soc::memory::ddr5::Ddr5Timing;

#[test]
fn cold_read_pays_act_plus_rcd_plus_cas_plus_burst() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let addr = addr_from(&cfg, 0, 0, 0, 0, 0);
    let id = h.issue(addr, 0, read_op());
    let resp = h.response_at(id);
    // ACT at 0 (cmd bus free), RD at t_rcd, data at t_rcd + t_cas,
    // burst end at + bl_half.
    assert_eq!(resp, t.t_rcd + t.t_cas + t.bl_half);
}

#[test]
fn row_hit_second_read_only_pays_ccd_plus_cas_plus_burst() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let a1 = addr_from(&cfg, 0, 0, 0, 0, 0);
    let a2 = addr_from(&cfg, 0, 0, 0, 0, 1);
    let first = h.issue(a1, 0, read_op());
    let _ = h.response_at(first);
    let second = h.issue(a2, 200, read_op());
    let resp = h.response_at(second);
    // Row already open. The controller re-issues from cycle 200 which is
    // well past the first read's data end; column cycle = 200, data end =
    // 200 + t_cas + bl_half.
    assert_eq!(resp, 200 + t.t_cas + t.bl_half);
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
    // From `start`: PRE (respect tRAS from ACT at cycle 0 — 77 is well
    // below 500 so PRE fires at `start`), tRP to next ACT, tRCD to
    // column, tCAS to data start, bl_half to data end.
    let expected = start + t.t_rp + t.t_rcd + t.t_cas + t.bl_half;
    assert_eq!(resp, expected);
}

#[test]
fn back_to_back_same_bank_group_reads_honor_ccd_l() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    // Same bank group, different banks — column stream still constrained
    // by tCCD_L when the last column was in the same BG.
    let a1 = addr_from(&cfg, 0, 0, 0, 0, 0);
    let a2 = addr_from(&cfg, 0, 0, 1, 0, 0);
    let id1 = h.issue(a1, 0, read_op());
    let end1 = h.response_at(id1);
    let id2 = h.issue(a2, end1, read_op());
    let end2 = h.response_at(id2);
    // Second read is to a new bank: ACT is needed. Column cycle >=
    // last_col + t_ccd_l. Take end1 -> last col was at end1 - bl_half - t_cas.
    let last_col = end1 - t.bl_half - t.t_cas;
    let expected_col = (last_col + t.t_ccd_l).max(end1);
    // First cmd from cycle end1: ACT (>= cmd_bus, >= t_rrd_l from prior ACT).
    // Column is bounded by max(act + t_rcd, ccd, cmd_bus).
    // Just assert that the resp cycle >= last_col + t_ccd_l + t_cas + bl_half.
    assert!(end2 >= expected_col + t.t_cas + t.bl_half);
}
