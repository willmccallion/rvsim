//! Rank power-down: an idle rank enters power-down and pays tXP on exit.

use crate::unit::soc::memory::ddr5::common::{Harness, addr_from, read_op, tiny_config};
use rvsim_core::sim::packet::DramCmdKind;
use rvsim_core::soc::memory::ddr5::{Ddr5Timing, PowerDownPolicy};

const IDLE_CLOCKS: u64 = 16;

#[test]
fn idle_rank_powers_down_and_the_next_activate_waits_t_xp_after_exit() {
    let mut cfg = tiny_config();
    cfg.power_down = PowerDownPolicy::AfterIdle { idle_clocks: IDLE_CLOCKS };
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let a = addr_from(&cfg, 0, 0, 0, 0, 0);
    let first = h.issue(a, 0, read_op());
    let first_end = h.response_at(first);
    h.run_until(first_end + 200);

    let entries = h.commands_of(DramCmdKind::PowerDownEntry);
    assert_eq!(entries.len(), 1, "rank should power down once idle");
    let reads = h.commands_of(DramCmdKind::Read);
    let burst_end = reads[0].fire_at + t.t_cas + t.bl_half;
    assert!(
        entries[0].fire_at >= burst_end && entries[0].fire_at >= reads[0].fire_at + IDLE_CLOCKS,
        "entry at {} precedes the idle window after the burst ending {burst_end}",
        entries[0].fire_at
    );

    let wake = first_end + 300;
    let second = h.issue(addr_from(&cfg, 0, 0, 0, 1, 0), wake, read_op());
    let _ = h.response_at(second);
    let exits = h.commands_of(DramCmdKind::PowerDownExit);
    assert_eq!(exits.len(), 1);
    assert_eq!(exits[0].fire_at, wake, "exit issues as soon as work arrives");
    let pre_all = h.commands_of(DramCmdKind::PrechargeAll);
    let precharges = h.commands_of(DramCmdKind::Precharge);
    let first_cmd_after_exit = precharges
        .iter()
        .chain(pre_all.iter())
        .map(|c| c.fire_at)
        .filter(|&f| f >= wake)
        .min()
        .expect("the open row must be closed before the new row opens");
    assert_eq!(first_cmd_after_exit, wake + t.t_xp, "first command waits tXP after exit");
}

#[test]
fn disabled_policy_never_issues_power_commands() {
    let cfg = tiny_config();
    let mut h = Harness::new(cfg);
    let a = addr_from(&cfg, 0, 0, 0, 0, 0);
    let first = h.issue(a, 0, read_op());
    let end = h.response_at(first);
    h.run_until(end + 1000);
    assert!(h.commands_of(DramCmdKind::PowerDownEntry).is_empty());
    assert!(h.commands_of(DramCmdKind::PowerDownExit).is_empty());
}

#[test]
fn a_rank_does_not_power_down_right_before_its_refresh() {
    let mut cfg = tiny_config();
    cfg.power_down = PowerDownPolicy::AfterIdle { idle_clocks: IDLE_CLOCKS };
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let a = addr_from(&cfg, 0, 0, 0, 0, 0);
    let id = h.issue(a, t.t_refi - t.t_pd - 4, read_op());
    let _ = h.response_at(id);
    h.run_until(t.t_refi + t.t_rfc1 + 100);
    let refreshes = h.commands_of(DramCmdKind::Refresh);
    assert_eq!(refreshes.len(), 1);
    for entry in h.commands_of(DramCmdKind::PowerDownEntry) {
        assert!(
            entry.fire_at + t.t_pd <= refreshes[0].fire_at || entry.fire_at > refreshes[0].fire_at,
            "power-down at {} would straddle the refresh at {}",
            entry.fire_at,
            refreshes[0].fire_at
        );
    }
}
