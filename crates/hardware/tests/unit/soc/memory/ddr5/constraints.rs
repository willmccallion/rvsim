//! tFAW / tWTR / tRTW / tRTP / tWR / tRTRS.

use crate::unit::soc::memory::ddr5::common::{
    Harness, addr_from, read_op, tiny_config, tworank_config, write_op,
};
use rvsim_core::soc::memory::ddr5::Ddr5Timing;

#[test]
fn fifth_activate_stalls_on_faw() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    // Four distinct (bg, bank) tuples => four ACTs land in the tFAW window,
    // plus a fifth ACT to the same bank[0] (different row) so PRE + ACT are
    // needed. All five are enqueued at cycle 0 to expose the tFAW stall.
    let acts = [
        addr_from(&cfg, 0, 0, 0, 0, 0),
        addr_from(&cfg, 0, 0, 1, 1, 0),
        addr_from(&cfg, 0, 1, 0, 2, 0),
        addr_from(&cfg, 0, 1, 1, 3, 0),
        addr_from(&cfg, 0, 0, 0, 5, 0),
    ];
    let mut ids = Vec::new();
    for a in &acts {
        ids.push(h.issue(*a, 0, read_op()));
    }
    for id in &ids {
        let _ = h.response_at(*id);
    }
    let cmds = h.take_dram_cmds();
    let acts_now: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c.kind, rvsim_core::sim::packet::DramCmdKind::Activate))
        .collect();
    assert!(acts_now.len() >= 5, "fewer than 5 ACTs recorded: {cmds:?}");
    let fifth_act = acts_now[4];
    // t_faw is 32; without stalling the fifth ACT could be earlier, but
    // tFAW forces it to >= faw[0] + t_faw = 0 + 32.
    assert!(
        fifth_act.fire_at >= t.t_faw,
        "5th ACT at {} < t_faw = {}",
        fifth_act.fire_at,
        t.t_faw
    );
}

#[test]
fn write_then_read_same_bank_group_pays_wtr_l() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let w = addr_from(&cfg, 0, 0, 0, 0, 0);
    let r = addr_from(&cfg, 0, 0, 0, 0, 1);
    let wid = h.issue(w, 0, write_op());
    let write_end = h.response_at(wid);
    let rid = h.issue(r, write_end, read_op());
    let read_end = h.response_at(rid);
    // Read column cycle >= write data end + t_wtr_l.
    let read_col_min = write_end + t.t_wtr_l;
    let expected_min = read_col_min + t.t_cas + t.bl_half;
    assert!(
        read_end >= expected_min,
        "read end {read_end} < expected min {expected_min} (write_end={write_end})"
    );
}

#[test]
fn read_then_write_pays_rtw() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let r = addr_from(&cfg, 0, 0, 0, 0, 0);
    let w = addr_from(&cfg, 0, 0, 0, 0, 1);
    let rid = h.issue(r, 0, read_op());
    let read_end = h.response_at(rid);
    let wid = h.issue(w, read_end, write_op());
    let _ = h.response_at(wid);
    let cmds = h.take_dram_cmds();
    let write_cmds: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c.kind, rvsim_core::sim::packet::DramCmdKind::Write))
        .collect();
    let first_write = write_cmds.first().expect("no WR cmd emitted");
    let last_read_cmd = read_end - t.t_cas - t.bl_half;
    assert!(
        first_write.fire_at >= last_read_cmd + t.t_rtw,
        "WR at {} < last RD cmd + t_rtw = {}",
        first_write.fire_at,
        last_read_cmd + t.t_rtw
    );
}

#[test]
fn rank_switch_pays_rtrs() {
    let cfg = tworank_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let r0 = addr_from(&cfg, 0, 0, 0, 0, 0);
    let r1 = addr_from(&cfg, 1, 0, 0, 0, 0);
    let id0 = h.issue(r0, 0, read_op());
    let end0 = h.response_at(id0);
    let id1 = h.issue(r1, end0, read_op());
    let _ = h.response_at(id1);
    let cmds = h.take_dram_cmds();
    let reads: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c.kind, rvsim_core::sim::packet::DramCmdKind::Read))
        .collect();
    assert!(reads.len() >= 2);
    // Both reads exist; the second is on rank 1 and its data start must
    // be >= end0 + t_rtrs.
    let second_rd = reads[1];
    let data_start = second_rd.fire_at + t.t_cas;
    assert!(
        data_start >= end0 + t.t_rtrs,
        "cross-rank data start {data_start} < prev data end + t_rtrs {}",
        end0 + t.t_rtrs
    );
}
