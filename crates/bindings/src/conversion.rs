//! Python↔Rust configuration conversion.
//!
//! Converts Python dicts (e.g., from `SimConfig.to_dict()`) into the core `Config` type
//! via JSON serialization, so the same schema is used from both Python and CLI.

use pyo3::prelude::*;
use riscv_core::config::Config;
use serde_json;

/// Converts a Python dict to a simulator `Config`.
///
/// The dict is serialized to JSON and then deserialized into `Config`. Keys must match
/// the Rust config structure (e.g., `general`, `system`, `memory`, `cache`, `pipeline`).
///
/// # Arguments
///
/// * `py` - Python interpreter handle.
/// * `dict` - A Python dict (e.g., from `riscv_sim.config.SimConfig.to_dict()`).
///
/// # Returns
///
/// The deserialized `Config`, or a `PyErr` if the dict is invalid.
pub fn py_dict_to_config(py: Python, dict: &Bound<'_, PyAny>) -> PyResult<Config> {
    let json = py.import("json")?;
    let dumps = json.getattr("dumps")?;
    let json_str_obj = dumps.call1((dict,))?;
    let json_str: String = json_str_obj.extract()?;

    let config: Config = serde_json::from_str(&json_str).map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid config: {}", e))
    })?;

    Ok(config)
}
