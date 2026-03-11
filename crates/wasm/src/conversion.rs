//! JS ↔ Rust configuration conversion.
//!
//! Converts a JS object (matching the Python `Config.to_dict()` shape) into
//! the core `Config` type via JSON serialization.

use rvsim_core::config::Config;
use wasm_bindgen::JsError;

/// Converts a JS value to a simulator `Config`.
///
/// The JS object is serialized to JSON via `serde-wasm-bindgen` and then
/// deserialized into `Config`.
pub fn js_to_config(val: wasm_bindgen::JsValue) -> Result<Config, JsError> {
    let json_str: String = js_sys::JSON::stringify(&val)
        .map_err(|_| JsError::new("Failed to stringify config object"))?
        .into();

    let config: Config = serde_json::from_str(&json_str)
        .map_err(|e| JsError::new(&format!("Invalid config: {e}")))?;

    Ok(config)
}
