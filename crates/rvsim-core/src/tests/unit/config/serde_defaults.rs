//! A configuration section the JSON leaves out, or leaves empty, takes the
//! values its Rust `Default` gives, so Python and Rust build the same
//! machine from the same partial config.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::{
    BhtConfig, CacheConfig, CoherenceConfig, GeneralConfig, IttageConfig, MemoryConfig,
    PerceptronConfig, PipelineConfig, StoreSetConfig, SystemConfig, TageConfig, TournamentConfig,
};

/// Checks that `{}` parses to `T::default()`.
fn assert_empty_section_is_default<T: DeserializeOwned + Serialize + Default>() {
    let parsed: T = serde_json::from_str("{}").expect("an empty section parses");
    assert_eq!(
        serde_json::to_value(parsed).expect("the parsed section serializes"),
        serde_json::to_value(T::default()).expect("the default serializes"),
        "{}",
        std::any::type_name::<T>()
    );
}

#[test]
fn an_empty_section_takes_its_struct_defaults() {
    assert_empty_section_is_default::<GeneralConfig>();
    assert_empty_section_is_default::<SystemConfig>();
    assert_empty_section_is_default::<MemoryConfig>();
    assert_empty_section_is_default::<CacheConfig>();
    assert_empty_section_is_default::<CoherenceConfig>();
    assert_empty_section_is_default::<PipelineConfig>();
    assert_empty_section_is_default::<StoreSetConfig>();
    assert_empty_section_is_default::<TageConfig>();
    assert_empty_section_is_default::<IttageConfig>();
    assert_empty_section_is_default::<BhtConfig>();
    assert_empty_section_is_default::<PerceptronConfig>();
    assert_empty_section_is_default::<TournamentConfig>();
}
