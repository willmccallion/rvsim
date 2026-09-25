//! Rank-wide refresh blocks that rank for tRFC; other ranks continue serving.

use crate::unit::soc::memory::ddr5::common::{
    Harness, addr_from, read_op, tiny_config, tworank_config,
};
use rvsim_core::sim::packet::DramCmdKind;
use rvsim_core::soc::memory::ddr5::Ddr5Timing;

#[test]
fn refresh_fires_after_t_refi_and_blocks_rank_for_t_rfc() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    // First access before t_refi — no refresh yet.
    let a = addr_from(&cfg, 0, 0, 0, 0, 0);
    let id1 = h.issue(a, 0, read_op());
    let _ = h.response_at(id1);
    let cmds1 = h.take_dram_cmds();
    assert!(!cmds1.iter().any(|c| matches!(c.kind, DramCmdKind::Refresh)));

    // Second access at cycle > t_refi triggers refresh before servicing.
    let id2 = h.issue(a, t.t_refi + 100, read_op());
    let end2 = h.response_at(id2);
    let cmds2 = h.take_dram_cmds();
    let refreshes: Vec<_> =
        cmds2.iter().filter(|c| matches!(c.kind, DramCmdKind::Refresh)).collect();
    assert_eq!(refreshes.len(), 1, "expected exactly one refresh, got {refreshes:?}");
    // Read completes only after tRFC has elapsed from the refresh start.
    let refresh_start = refreshes[0].fire_at;
    assert!(
        end2 >= refresh_start + t.t_rfc1 + t.t_rcd + t.t_cas + t.bl_half,
        "read after refresh completed too early: end={end2}, refresh_start={refresh_start}"
    );
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
