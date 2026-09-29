//! Python↔Rust configuration conversion.
//!
//! Converts Python dicts (e.g., from `SimConfig.to_dict()`) into the core `Config` type
//! via JSON serialization, so the same schema is used from both Python and CLI.

use pyo3::prelude::*;
use rvsim_core::config::Config;
use serde_json;

/// Converts a Python dict to a simulator `Config` via JSON serialization.
pub fn py_dict_to_config(py: Python<'_>, dict: &Bound<'_, PyAny>) -> PyResult<Config> {
    let json = py.import("json")?;
    let dumps = json.getattr("dumps")?;
    let json_str_obj = dumps.call1((dict,))?;
    let json_str: String = json_str_obj.extract()?;

    let config: Config = serde_json::from_str(&json_str).map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid config: {e}"))
    })?;

    Ok(config)
}
