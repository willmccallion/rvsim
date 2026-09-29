//! Tests for the checks `Config::validate` applies before a system is built.

use rvsim_core::config::{Config, ConfigError};

#[test]
fn default_config_validates() {
    assert!(Config::default().validate().is_ok());
}

#[test]
fn btb_with_a_power_of_two_set_count_validates() {
    let mut config = Config::default();
    config.pipeline.btb_size = 16384;
    config.pipeline.btb_ways = 8;

    assert!(config.validate().is_ok());
}

#[test]
fn btb_whose_set_count_is_not_a_power_of_two_is_rejected() {
    let mut config = Config::default();
    config.pipeline.btb_size = 12288;
    config.pipeline.btb_ways = 8;

    let err = config.validate().unwrap_err();

    assert!(matches!(err, ConfigError::BtbSets { size: 12288, ways: 8, sets: 1536 }), "{err}");
}

#[test]
fn a_simple_controller_bandwidth_that_is_not_positive_is_rejected() {
    let mut config = Config::default();
    config.memory.simple_bandwidth_gib_s = 0.0;

    let err = config.validate().unwrap_err();

    assert!(matches!(err, ConfigError::SimpleBandwidth), "{err}");
}
