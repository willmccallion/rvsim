//! tFAW / tWTR / tRTW / tRTRS.

use crate::config::ddr5::Ddr5Timing;
use crate::sim::packet::DramCmdKind;
use crate::tests::unit::soc::memory::ddr5::common::{
    Harness, addr_from, read_op, tiny_config, tworank_config, write_op,
};

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
    let acts_now = h.commands_of(DramCmdKind::Activate);
    assert!(acts_now.len() >= 5, "fewer than 5 ACTs recorded: {acts_now:?}");
    let fifth_act = acts_now[4];
    assert!(
        fifth_act.fire_at >= acts_now[0].fire_at + t.t_faw,
        "5th ACT at {} < first ACT + t_faw = {}",
        fifth_act.fire_at,
        acts_now[0].fire_at + t.t_faw
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
    let _ = h.response_at(wid);
    // Let the write drain to the bank, then queue a read to another line of
    // the same row while the write burst is still on the data bus.
    let rid = h.issue(r, t.t_rcd + 1, read_op());
    let _ = h.response_at(rid);
    let writes = h.commands_of(DramCmdKind::Write);
    let reads = h.commands_of(DramCmdKind::Read);
    let write_end = writes[0].fire_at + t.t_cwl + t.bl_half;
    assert!(
        reads[0].fire_at >= write_end + t.t_wtr_l,
        "RD at {} < write burst end + t_wtr_l = {}",
        reads[0].fire_at,
        write_end + t.t_wtr_l
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
    let wid = h.issue(w, 1, write_op());
    let _ = h.response_at(rid);
    let _ = h.response_at(wid);
    h.run_until(400);
    let reads = h.commands_of(DramCmdKind::Read);
    let writes = h.commands_of(DramCmdKind::Write);
    let read_burst_end = reads[0].fire_at + t.t_cas + t.bl_half;
    let write_burst_start = writes[0].fire_at + t.t_cwl;
    assert!(
        write_burst_start >= read_burst_end + t.t_rtw,
        "WR data at {write_burst_start} < read burst end + t_rtw = {}",
        read_burst_end + t.t_rtw
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
    let id1 = h.issue(r1, 0, read_op());
    let _ = h.response_at(id0);
    let _ = h.response_at(id1);
    let reads = h.commands_of(DramCmdKind::Read);
    assert_eq!(reads.len(), 2);
    let first_end = reads[0].fire_at + t.t_cas + t.bl_half;
    let second_start = reads[1].fire_at + t.t_cas;
    assert!(
        second_start >= first_end + t.t_rtrs,
        "cross-rank data start {second_start} < prev data end + t_rtrs {}",
        first_end + t.t_rtrs
    );
}
