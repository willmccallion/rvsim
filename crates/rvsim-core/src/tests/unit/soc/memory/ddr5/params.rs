//! Configuration parameters: JSON deserialization, timing overrides, and
//! validation of the DDR5 parameter block.

use crate::config::ddr5::{
    Ddr5Params, Ddr5Timing, EccKind, PowerDownPolicy, RefreshKind, SchedulerKind,
};
use crate::config::{Config, MemoryConfig, MemoryControllerKind};

#[test]
fn defaults_match_the_built_in_ddr5_4800_configuration() {
    let params: Ddr5Params = serde_json::from_str("{}").unwrap();
    let config = params.to_config();
    assert_eq!(config, crate::config::ddr5::Ddr5Config::default());
}

#[test]
fn a_config_file_selects_bin_policies_and_timing_overrides() {
    let json = r#"{
        "speed_bin": "5600B",
        "channels": 1,
        "ranks_per_channel": 1,
        "scheduler": "Fcfs",
        "refresh": "SameBank",
        "address_mapping": "RoRaBaCoCh",
        "power_down_idle_ns": 100,
        "ecc": "SecDed",
        "patrol_scrub_ns": 50000,
        "frontend_latency_ns": 5,
        "timing": {"t_rcd": 50, "t_faw": 40}
    }"#;
    let params: Ddr5Params = serde_json::from_str(json).unwrap();
    let config = params.to_config();
    let bin = Ddr5Timing::from_bin(&crate::config::ddr5::Ddr5SpeedBin::DDR5_5600B);
    assert_eq!(config.timing.data_rate_mts, 5600);
    assert_eq!(config.timing.t_cas, bin.t_cas);
    assert_eq!(config.timing.t_rcd, 50, "override applied");
    assert_eq!(config.timing.t_faw, 40, "override applied");
    assert_eq!(config.timing.t_rp, bin.t_rp, "untouched fields keep the bin value");
    assert_eq!(config.channels, 1);
    assert_eq!(config.scheduler, SchedulerKind::Fcfs);
    assert_eq!(config.refresh, RefreshKind::SameBank);
    assert_eq!(config.power_down, PowerDownPolicy::AfterIdle { idle_clocks: 280 });
    assert_eq!(config.ecc, EccKind::SecDed { patrol_scrub_ns: Some(50_000) });
    assert_eq!(config.frontend_latency, 14, "5 ns at 2.8 GHz");
    assert_eq!(config.backend_latency, 28, "default 10 ns at 2.8 GHz");
}

#[test]
fn non_power_of_two_topology_is_rejected_at_parse_time() {
    let err = serde_json::from_str::<Ddr5Params>(r#"{"channels": 3}"#).unwrap_err();
    assert!(err.to_string().contains("ddr5.channels must be a power of two"), "{err}");
    let err = serde_json::from_str::<Ddr5Params>(r#"{"write_low_watermark": 60}"#).unwrap_err();
    assert!(err.to_string().contains("write watermarks"), "{err}");
    let err = serde_json::from_str::<Ddr5Params>(r#"{"timing": {"t_bogus": 1}}"#).unwrap_err();
    assert!(err.to_string().contains("unknown variant"), "{err}");
}

#[test]
fn the_memory_section_carries_the_ddr5_block() {
    let json = r#"{"controller": "Ddr5", "ddr5": {"speed_bin": "4800B", "ranks_per_channel": 4}}"#;
    let memory: MemoryConfig = serde_json::from_str(json).unwrap();
    assert_eq!(memory.controller, MemoryControllerKind::Ddr5);
    assert_eq!(memory.ddr5.to_config().ranks_per_channel, 4);
    let default = Config::default();
    assert_eq!(default.system.cpu_clock_mhz, 2400, "1:1 with the DDR5-4800 command clock");
}
