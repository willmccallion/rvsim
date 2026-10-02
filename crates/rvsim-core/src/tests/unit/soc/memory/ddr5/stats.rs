//! Controller statistics: command counts, row-buffer hit rate, bus
//! utilisation and latency samples land in the stats tree under
//! `memctrl0.ch<C>.sc<S>`.

use crate::config::ddr5::Ddr5Timing;
use crate::tests::unit::soc::memory::ddr5::common::{
    Harness, addr_from, read_op, tiny_config, write_op,
};

#[test]
fn counters_and_derived_rates_reflect_the_commands_issued() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::new(cfg);
    let miss = h.issue(addr_from(&cfg, 0, 0, 0, 0, 0), 0, read_op());
    let _ = h.response_at(miss);
    let hit = h.issue(addr_from(&cfg, 0, 0, 0, 0, 1), 200, read_op());
    let _ = h.response_at(hit);
    let write = h.issue(addr_from(&cfg, 0, 0, 0, 0, 2), 400, write_op());
    let merged = h.issue(addr_from(&cfg, 0, 0, 0, 0, 2), 400, write_op());
    let _ = h.response_at(write);
    let _ = h.response_at(merged);
    h.run_until(900);

    let get = |path: &str| h.stats.get(path).unwrap_or_else(|| panic!("missing stat {path}"));
    assert_eq!(get("memctrl0.ch0.sc0.reads"), 2.0);
    assert_eq!(get("memctrl0.ch0.sc0.writes"), 1.0);
    assert_eq!(get("memctrl0.ch0.sc0.writes_merged"), 1.0);
    assert_eq!(get("memctrl0.ch0.sc0.activates"), 1.0);
    assert_eq!(get("memctrl0.ch0.sc0.row_misses"), 1.0);
    assert_eq!(get("memctrl0.ch0.sc0.row_hits"), 2.0, "the row-hit read and the write");
    assert!((get("memctrl0.ch0.sc0.row_hit_rate") - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(get("memctrl0.ch0.sc0.bus_busy_clocks"), 3.0 * t.bl_half as f64);
    assert!(get("memctrl0.ch0.sc0.clocks") >= 900.0);
    assert!(get("memctrl0.ch0.sc0.data_bus_utilization") > 0.0);
    assert_eq!(get("memctrl0.ch0.sc0.refreshes"), 0.0);
    assert_eq!(get("memctrl0.ch0.sc0.rank0.bank0.reads"), 2.0);
    assert_eq!(get("memctrl0.ch0.sc0.rank0.bank0.writes"), 1.0);
    assert_eq!(get("memctrl0.ch0.sc0.rank0.bank0.activates"), 1.0);

    let latency = h.stats.histogram("memctrl0.ch0.sc0.read_latency");
    assert_eq!(latency.count(), 2);
    assert_eq!(latency.min(), Some(t.t_cas + t.bl_half), "row hit: column at arrival");
    assert_eq!(latency.max(), Some(t.t_rcd + t.t_cas + t.bl_half), "row miss: ACT first");

    let summary = h.stats.summary(0, 0);
    assert!(summary.contains("[memctrl0]"));
    assert!(summary.contains("ch0.sc0.row_hit_rate"));
    assert!(!summary.contains("rank0.bank0"), "bank counters stay out of the summary");
}
