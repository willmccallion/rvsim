//! Python↔Rust configuration conversion.
//!
//! Converts Python dicts (e.g., from `Config.to_dict()`) to and from the core
//! `Config` type via JSON, so both sides read and write one schema.

use pyo3::exceptions::PyValueError;
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

    let mut deserializer = serde_json::Deserializer::from_str(&json_str);
    let config: Config = serde_path_to_error::deserialize(&mut deserializer).map_err(|e| {
        let message = without_position(e.inner());
        if e.path().iter().next().is_none() {
            PyValueError::new_err(format!("Invalid config: {message}"))
        } else {
            PyValueError::new_err(format!("Invalid config at {}: {message}", e.path()))
        }
    })?;
    deserializer
        .end()
        .map_err(|e| PyValueError::new_err(format!("Invalid config: {}", without_position(&e))))?;

    Ok(config)
}

/// A JSON error's message without its line and column, which point into
/// JSON the caller never sees.
fn without_position(error: &serde_json::Error) -> String {
    let message = error.to_string();
    let position = format!(" at line {} column {}", error.line(), error.column());
    if let Some(stripped) = message.strip_suffix(&position) {
        return stripped.to_owned();
    }
    message
}

/// Converts a configuration value, a whole `Config` or a component of one,
/// to the Python dict `py_dict_to_config` reads it from.
///
/// # Errors
///
/// Returns `ValueError` if the value cannot be written as JSON.
pub fn config_to_py_dict<T: serde::Serialize>(py: Python<'_>, value: &T) -> PyResult<PyObject> {
    let json_str = serde_json::to_string(value)
        .map_err(|e| PyValueError::new_err(format!("Unwritable config: {e}")))?;
    let json = py.import("json")?;
    Ok(json.getattr("loads")?.call1((json_str,))?.unbind())
}
