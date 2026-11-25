//! LPDDR4 on the command-level controller: the speed bin resolves to the
//! datasheet's rules, a package is two x16 channels of eight banks, and the
//! controller spaces commands as the datasheet's command-to-command table
//! requires.

use crate::config::dram::{DramStandardConfig, RefreshKind};
use crate::config::lpddr4::{LPDDR4_COMMAND_CYCLES, Lpddr4Density, Lpddr4Params, Lpddr4SpeedBin};
use crate::config::{MemoryConfig, MemoryControllerKind};
use crate::sim::packet::DramCmdKind;
use crate::tests::unit::soc::memory::ddr5::common::{
    Harness, addr_from, make_controller_with_clock, read_op, tiny_config, write_op,
};

/// Core clock equal to LPDDR4-3200's 1600 MHz command clock, so cycles
/// read as DRAM clocks.
const LPDDR4_3200_COMMAND_MHZ: u64 = 1600;
/// LPDDR4-3200's WRITE latency (set A) and READ latency, in clocks.
const WL: u64 = 14;
const RL: u64 = 28;
/// Half a 32-beat burst: the clocks a 64-byte line takes on a channel.
const BL_HALF: u64 = 16;

fn lpddr4() -> DramStandardConfig {
    serde_json::from_str::<Lpddr4Params>("{}").expect("default LPDDR4 parameters").to_config()
}

fn harness(cfg: &DramStandardConfig) -> Harness {
    Harness::with_controller(make_controller_with_clock(cfg, LPDDR4_3200_COMMAND_MHZ))
}

#[test]
fn lpddr4_3200_resolves_its_rules_from_the_first_clock_of_each_command() {
    let t = Lpddr4SpeedBin::LPDDR4_3200.timing(Lpddr4Density::Gb16);

    assert_eq!(t.t_cas, RL + 3 + 6, "RL from CAS-2's last clock, plus tDQSCK max of 3.5 ns");
    assert_eq!(t.t_cwl, WL + 3 + 1, "WL from CAS-2's last clock, plus tDQSS");
    assert_eq!(t.t_rcd, 29 + 2, "18 ns from ACTIVATE-2");
    assert_eq!(t.t_ras, 68 + 2, "42 ns from ACTIVATE-2");
    assert_eq!((t.t_rp, t.t_rp_all), (29, 34), "18 ns per bank, 21 ns all banks");
    assert_eq!(t.t_rc, t.t_ras + t.t_rp);
    assert_eq!((t.t_rrd_s, t.t_rrd_l, t.t_faw), (16, 16, 64), "10 ns, no bank groups, 40 ns");
    assert_eq!((t.t_ccd_s, t.t_ccd_l, t.t_ccd_l_wr), (BL_HALF, BL_HALF, BL_HALF));
    assert_eq!(t.t_ppd, 4);
    assert_eq!((t.t_rfc1, t.t_rfcsb, t.t_refi), (608, 304, 6247), "380 ns, 190 ns, 3.904 us");
    assert_eq!(t.bl_half, BL_HALF);
}

#[test]
fn a_package_is_two_x16_channels_of_eight_banks_and_2_kib_rows() {
    let cfg = lpddr4();

    assert_eq!((cfg.channels, cfg.subchannels_per_channel, cfg.ranks_per_channel), (2, 1, 1));
    assert_eq!((cfg.bank_groups_per_rank, cfg.banks_per_group.get()), (1, 8));
    assert_eq!((cfg.row_bits, cfg.row_size_bytes), (17, 2048));
    assert_eq!(cfg.commands, LPDDR4_COMMAND_CYCLES);
    assert_eq!(cfg.timing.data_rate_mts, 3200);
}

#[test]
fn a_smaller_die_has_fewer_rows_and_refreshes_faster() {
    let params: Lpddr4Params = serde_json::from_str(r#"{"density": "Gb8"}"#).unwrap();

    let cfg = params.to_config();

    assert_eq!(cfg.row_bits, 16);
    assert_eq!((cfg.timing.t_rfc1, cfg.timing.t_rfcsb), (448, 224), "280 ns and 140 ns");
}

#[test]
fn per_bank_refresh_refreshes_one_bank_at_a_time() {
    let params: Lpddr4Params = serde_json::from_str(r#"{"refresh": "PerBank"}"#).unwrap();

    assert_eq!(params.to_config().refresh, RefreshKind::SameBank);
}

#[test]
fn a_topology_count_that_is_not_a_power_of_two_is_refused_by_name() {
    let err = serde_json::from_str::<Lpddr4Params>(r#"{"channels": 3}"#).unwrap_err();
    assert!(err.to_string().contains("lpddr4.channels must be a power of two"), "{err}");
    let err = serde_json::from_str::<Lpddr4Params>(r#"{"bank_groups_per_rank": 2}"#).unwrap_err();
    assert!(err.to_string().contains("unknown field"), "{err}");
}

#[test]
fn the_memory_section_carries_the_lpddr4_block() {
    let json = r#"{"controller": "Lpddr4", "lpddr4": {"ranks_per_channel": 2}}"#;

    let memory: MemoryConfig = serde_json::from_str(json).unwrap();

    assert_eq!(memory.controller, MemoryControllerKind::Lpddr4);
    assert_eq!(memory.lpddr4.to_config().ranks_per_channel, 2);
}

#[test]
fn an_idle_read_activates_then_reads_after_trcd_and_returns_after_rl() {
    let cfg = lpddr4();
    let t = cfg.timing;
    let mut h = harness(&cfg);

    let id = h.issue(addr_from(&cfg, 0, 0, 0, 0, 0), 0, read_op());
    let answered = h.response_at(id);

    let act = h.commands_of(DramCmdKind::Activate)[0].fire_at;
    let read = h.commands_of(DramCmdKind::Read)[0].fire_at;
    assert_eq!(read, act + t.t_rcd);
    let latencies = cfg.frontend_latency + cfg.backend_latency;
    assert_eq!(answered, read + t.t_cas + BL_HALF + latencies);
}

#[test]
fn reads_of_an_open_row_follow_one_burst_apart() {
    let cfg = lpddr4();
    let mut h = harness(&cfg);
    let ids: Vec<_> =
        (0..2).map(|column| h.issue(addr_from(&cfg, 0, 0, 0, 0, column), 0, read_op())).collect();
    for id in ids {
        let _ = h.response_at(id);
    }

    let reads = h.commands_of(DramCmdKind::Read);

    assert_eq!(reads[1].fire_at - reads[0].fire_at, BL_HALF, "tCCD for BL32");
}

#[test]
fn a_read_follows_a_write_by_the_datasheets_write_to_read_spacing() {
    let cfg = lpddr4();
    let mut h = harness(&cfg);
    let wid = h.issue(addr_from(&cfg, 0, 0, 0, 0, 0), 0, write_op());
    let _ = h.response_at(wid);
    let rid = h.issue(addr_from(&cfg, 0, 0, 0, 0, 1), cfg.timing.t_rcd + 1, read_op());
    let _ = h.response_at(rid);

    let write = h.commands_of(DramCmdKind::Write)[0].fire_at;
    let read = h.commands_of(DramCmdKind::Read)[0].fire_at;

    // WL + 1 + BL/2 + RU(tWTR / tCK), tWTR = MAX(10 ns, 8 nCK) = 16.
    assert!(read >= write + WL + 1 + BL_HALF + 16, "RD at {read}, WR at {write}");
}

#[test]
fn a_write_follows_a_read_by_the_datasheets_read_to_write_spacing() {
    let cfg = lpddr4();
    let mut h = harness(&cfg);
    let rid = h.issue(addr_from(&cfg, 0, 0, 0, 0, 0), 0, read_op());
    let wid = h.issue(addr_from(&cfg, 0, 0, 0, 0, 1), 1, write_op());
    let _ = h.response_at(rid);
    let _ = h.response_at(wid);
    h.run_until(600);

    let read = h.commands_of(DramCmdKind::Read)[0].fire_at;
    let write = h.commands_of(DramCmdKind::Write)[0].fire_at;

    // RL + RU(tDQSCKmax / tCK) + BL/2 - WL + tWPRE: 28 + 6 + 16 - 14 + 2.
    assert!(write >= read + RL + 6 + BL_HALF - WL + 2, "WR at {write}, RD at {read}");
}

#[test]
fn a_row_closes_no_sooner_than_the_datasheet_allows_after_a_read_or_a_write() {
    let cfg = lpddr4();
    let t = cfg.timing;
    for (op, after_column) in [
        // READ to PRECHARGE: BL/2 + MAX(8, RU(tRTP / tCK)) - 8.
        (read_op(), BL_HALF + 12 - 8),
        // WRITE to PRECHARGE: WL + 1 + BL/2 + RU(tWR / tCK), tWR = 29.
        (write_op(), WL + 1 + BL_HALF + 29),
    ] {
        let mut h = harness(&cfg);
        let first = h.issue(addr_from(&cfg, 0, 0, 0, 0, 0), 0, op);
        let _ = h.response_at(first);
        let other_row = h.issue(addr_from(&cfg, 0, 0, 0, 1, 0), t.t_rcd + 1, read_op());
        let _ = h.response_at(other_row);

        let column = h
            .dram_cmds()
            .into_iter()
            .find(|c| matches!(c.kind, DramCmdKind::Read | DramCmdKind::Write))
            .expect("a column command")
            .fire_at;
        let precharge = h.commands_of(DramCmdKind::Precharge)[0].fire_at;

        assert!(precharge >= column + after_column, "PRE at {precharge}, column at {column}");
    }
}

#[test]
fn a_command_holds_the_command_bus_for_its_standards_clocks() {
    let mut cfg = tiny_config();
    cfg.commands = LPDDR4_COMMAND_CYCLES;
    cfg.timing.t_ccd_s = 1;
    cfg.timing.t_ccd_l = 1;
    cfg.timing.bl_half = 1;
    let mut h = Harness::new(&cfg);
    let ids = [0, 1].map(|column| h.issue(addr_from(&cfg, 0, 0, 0, 0, column), 0, read_op()));
    for id in ids {
        let _ = h.response_at(id);
    }

    let reads = h.commands_of(DramCmdKind::Read);

    assert_eq!(reads[1].fire_at - reads[0].fire_at, LPDDR4_COMMAND_CYCLES.column);
}
