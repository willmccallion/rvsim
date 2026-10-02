//! Refresh: a due refresh precharges the covered banks, issues REFRESH after
//! tRP, and holds those banks for tRFC while everything else keeps serving.

use crate::config::ddr5::{Ddr5Timing, RefreshKind};
use crate::sim::packet::DramCmdKind;
use crate::soc::memory::ddr5::refresh::{RankLayout, bank_mask_all, bank_mask_set};
use crate::tests::unit::soc::memory::ddr5::common::{
    Harness, addr_from, read_op, tiny_config, tworank_config,
};

#[test]
fn all_bank_refresh_closes_open_rows_then_holds_the_rank_for_t_rfc() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    // Open row 0 of bank 0 and leave it open.
    let a = addr_from(&cfg, 0, 0, 0, 0, 0);
    let id1 = h.issue(a, 0, read_op());
    let _ = h.response_at(id1);
    assert!(h.commands_of(DramCmdKind::Refresh).is_empty());

    // A read to the open row just after the refresh becomes due must wait
    // for PRECHARGE-ALL, REFRESH, tRFC, and a fresh ACTIVATE.
    let id2 = h.issue(a, t.t_refi + 100, read_op());
    let end2 = h.response_at(id2);

    let pre_all = h.commands_of(DramCmdKind::PrechargeAll);
    let refreshes = h.commands_of(DramCmdKind::Refresh);
    assert_eq!(pre_all.len(), 1, "one PRECHARGE-ALL for the open row");
    assert_eq!(refreshes.len(), 1, "exactly one refresh, got {refreshes:?}");
    assert_eq!(pre_all[0].fire_at, t.t_refi, "refresh becomes due at tREFI");
    assert_eq!(refreshes[0].fire_at, t.t_refi + t.t_rp, "REFRESH waits tRP after the precharge");
    let acts = h.commands_of(DramCmdKind::Activate);
    assert_eq!(acts.len(), 2, "the row had to be reopened after the refresh");
    assert!(
        acts[1].fire_at >= refreshes[0].fire_at + t.t_rfc1,
        "ACT at {} inside tRFC1 window ending {}",
        acts[1].fire_at,
        refreshes[0].fire_at + t.t_rfc1
    );
    assert!(end2 >= refreshes[0].fire_at + t.t_rfc1 + t.t_rcd + t.t_cas + t.bl_half);
}

#[test]
fn refresh_does_not_block_other_rank() {
    let cfg = tworank_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    // Enqueue both requests at the same simulated moment before letting
    // the scheduler tick — rank 0's refresh window must not force rank 1's
    // read to serialize behind it.
    let a0 = addr_from(&cfg, 0, 0, 0, 0, 0);
    let a1 = addr_from(&cfg, 1, 0, 0, 0, 0);
    let id0 = h.issue(a0, t.t_refi + 100, read_op());
    let id1 = h.issue(a1, t.t_refi + 100, read_op());
    let end1 = h.response_at(id1);
    let end0 = h.response_at(id0);
    // rank 1 should finish faster than rank 0 (which paid refresh).
    assert!(end1 < end0, "rank1 end {end1} >= rank0 end {end0}");
}

#[test]
fn same_bank_refresh_rotates_bank_sets_and_leaves_the_others_serving() {
    let mut cfg = tiny_config();
    cfg.refresh = RefreshKind::SameBank;
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    // Two banks per group: set 0 refreshes at tREFI/2, set 1 at tREFI.
    let interval = t.t_refi / 2;
    let in_set0 = addr_from(&cfg, 0, 0, 0, 0, 0);
    let in_set1 = addr_from(&cfg, 0, 0, 1, 0, 0);
    let held = h.issue(in_set0, interval + 1, read_op());
    let free = h.issue(in_set1, interval + 1, read_op());
    let end_free = h.response_at(free);
    let end_held = h.response_at(held);
    let refreshes = h.commands_of(DramCmdKind::Refresh);
    assert_eq!(refreshes.len(), 1);
    assert_eq!(refreshes[0].bank, 0, "first REFsb covers bank set 0");
    assert!(
        end_free < refreshes[0].fire_at + t.t_rfcsb,
        "set-1 read at {end_free} waited for set-0's refresh ending {}",
        refreshes[0].fire_at + t.t_rfcsb
    );
    assert!(end_held >= refreshes[0].fire_at + t.t_rfcsb + t.t_rcd + t.t_cas + t.bl_half);

    h.run_until(t.t_refi + 400);
    let refreshes = h.commands_of(DramCmdKind::Refresh);
    assert_eq!(refreshes.len(), 2);
    assert_eq!(refreshes[1].bank, 1, "second REFsb covers bank set 1");
    assert!(refreshes[1].fire_at >= t.t_refi);
}

#[test]
fn bank_masks_cover_the_rank_and_one_bank_per_group() {
    let layout = RankLayout { bank_groups: 2, banks_per_group: 2 };
    assert_eq!(bank_mask_all(layout), 0b1111);
    assert_eq!(bank_mask_set(layout, 0), 0b0101);
    assert_eq!(bank_mask_set(layout, 1), 0b1010);
    let wide = RankLayout { bank_groups: 8, banks_per_group: 4 };
    assert_eq!(bank_mask_all(wide), (1u64 << 32) - 1);
    assert_eq!(bank_mask_set(wide, 3).count_ones(), 8);
}

#[test]
fn a_controller_resumed_later_starts_fresh_there_instead_of_replaying_the_gap() {
    use crate::soc::memory::controller::MemoryController;
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let a = addr_from(&cfg, 0, 0, 0, 0, 0);
    let mut fresh = Harness::new(cfg);
    let fresh_id = fresh.issue(a, 0, read_op());
    let fresh_latency = fresh.response_at(fresh_id);
    let resume = 5 * t.t_refi;
    let mut h = Harness::new(cfg);

    h.controller.resume_at(resume);
    h.next_tick = resume;
    let id = h.issue(a, resume, read_op());
    let latency = h.response_at(id) - resume;
    h.run_until(resume + t.t_refi + t.t_rp + 1);

    assert_eq!(latency, fresh_latency, "the first read sees a freshly powered-up DRAM");
    let refresh_starts: Vec<u64> = h
        .dram_cmds()
        .into_iter()
        .filter(|c| matches!(c.kind, DramCmdKind::PrechargeAll | DramCmdKind::Refresh))
        .map(|c| c.fire_at)
        .collect();
    assert_eq!(refresh_starts.first(), Some(&(resume + t.t_refi)), "{refresh_starts:?}");
    assert_eq!(h.commands_of(DramCmdKind::Refresh).len(), 1, "no refreshes owed from the gap");
}
