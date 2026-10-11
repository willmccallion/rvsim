//! Python↔Rust configuration conversion.
//!
//! Converts Python dicts (e.g., from `Config.to_dict()`) to and from the core
//! `Config` type via JSON, so both sides read and write one schema.

use pyo3::prelude::*;
use rvsim_core::config::Config;
use serde_json;

/// Converts a Python dict to a simulator `Config` via JSON serialization.
///
/// # Errors
///
/// Returns `ValueError` when the dict is not a configuration the core
/// accepts, naming the field at fault.
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

/// Converts a simulator `Config` to the Python dict `py_dict_to_config`
/// reads.
///
/// # Errors
///
/// Returns `ValueError` if the configuration cannot be written as JSON.
pub fn config_to_py_dict(py: Python<'_>, config: &Config) -> PyResult<PyObject> {
    let json_str = serde_json::to_string(config).map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Unwritable config: {e}"))
    })?;
    let json = py.import("json")?;
    Ok(json.getattr("loads")?.call1((json_str,))?.unbind())
}
