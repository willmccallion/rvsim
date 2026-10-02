//! Core-to-DRAM clock conversion.

use crate::config::ddr5::Ddr5Timing;
use crate::soc::memory::ddr5::ClockRatio;
use crate::tests::unit::soc::memory::ddr5::common::{
    Harness, addr_from, controller_latency, make_controller_with_clock, read_op, tiny_config,
};

#[test]
fn conversions_round_dram_clocks_down_and_core_cycles_up() {
    let clock = ClockRatio::new(3000, 4800);
    assert_eq!(clock.to_dram(0), 0);
    assert_eq!(clock.to_dram(5), 4, "5 core cycles at 3 GHz are 4 DRAM clocks at 2.4 GHz");
    assert_eq!(clock.to_dram(1250), 1000);
    assert_eq!(clock.to_cpu(4), 5);
    assert_eq!(clock.to_cpu(1000), 1250);
    assert_eq!(clock.to_cpu(1), 2, "one DRAM clock is 1.25 core cycles, rounded up");
}

#[test]
fn one_to_one_clock_is_the_identity() {
    let clock = ClockRatio::new(2400, 4800);
    for c in [0, 1, 7, 9360] {
        assert_eq!(clock.to_dram(c), c);
        assert_eq!(clock.to_cpu(c), c);
    }
}

#[test]
fn faster_core_sees_dram_latency_scaled_into_its_own_cycles() {
    let cfg = tiny_config();
    let t = Ddr5Timing::default();
    let mut h = Harness::with_controller(make_controller_with_clock(cfg, 4800));
    let addr = addr_from(&cfg, 0, 0, 0, 0, 0);
    let id = h.issue(addr, 0, read_op());
    let resp = h.response_at(id);
    let dram_clocks = t.t_rcd + t.t_cas + t.bl_half + controller_latency(&cfg);
    assert_eq!(resp, 2 * dram_clocks, "at 4.8 GHz every DRAM clock is two core cycles");
}
