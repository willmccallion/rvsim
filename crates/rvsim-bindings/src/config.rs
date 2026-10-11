//! The core's machines and its configuration check, as the nested dicts
//! a simulator takes.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rvsim_core::config::Config;

use crate::conversion::{config_to_py_dict, py_dict_to_config};

/// The default machine, which the Python API's `Config()` describes.
///
/// # Errors
///
/// Fails only if the configuration cannot be written as JSON.
#[pyfunction]
pub fn default_config(py: Python<'_>) -> PyResult<PyObject> {
    config_to_py_dict(py, &Config::default())
}

/// The minimal machine: every component at its own default, one-wide and
/// in order, with no caches and fixed-latency memory.
///
/// # Errors
///
/// Fails only if the configuration cannot be written as JSON.
#[pyfunction]
pub fn minimal_config(py: Python<'_>) -> PyResult<PyObject> {
    config_to_py_dict(py, &Config::minimal())
}

/// `config_dict` as the core reads it, with every field it has, once the
/// core has checked it.
///
/// # Errors
///
/// A ``ValueError`` when the dict is not a configuration or describes a
/// machine the core refuses to build.
#[pyfunction]
pub fn check_config(py: Python<'_>, config_dict: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    let config = py_dict_to_config(py, config_dict)?;
    config.validate().map_err(|e| PyValueError::new_err(e.to_string()))?;
    config_to_py_dict(py, &config)
}
