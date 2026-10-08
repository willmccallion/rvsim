//! Tests for the checks `Config::validate` applies before a system is built.

use crate::config::{CacheConfig, Config, ConfigError};

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

#[test]
fn a_cache_with_zero_mshrs_write_buffers_or_targets_is_refused() {
    for field in ["mshr_count", "write_buffers", "targets_per_mshr"] {
        let parsed = serde_json::from_str::<CacheConfig>(&format!(r#"{{"{field}": 0}}"#));

        let error = parsed.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(error.contains("nonzero"), "{field} = 0: {error:?}");
    }
}

#[test]
fn an_omitted_cache_resource_count_takes_its_default() {
    let cache: CacheConfig = serde_json::from_str(r#"{"size_bytes": 8192}"#).unwrap();

    assert_eq!(
        (cache.mshr_count.get(), cache.write_buffers.get(), cache.targets_per_mshr.get()),
        (8, 8, 20)
    );
}
