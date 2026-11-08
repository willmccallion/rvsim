//! The core's machines and its configuration check, as the nested dicts
//! a simulator takes.

use pyo3::prelude::*;
use rvsim_core::config::{ComponentDefaults, Config};

use crate::conversion::{config_from_py, config_to_py_dict};

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

/// Every configuration component at its own default, by name: what each
/// builder of the Python API starts from.
///
/// # Errors
///
/// Fails only if a component cannot be written as JSON.
#[pyfunction]
pub fn component_defaults(py: Python<'_>) -> PyResult<PyObject> {
    config_to_py_dict(py, &ComponentDefaults::default())
}

/// The ISA string the configuration implements, as the device tree
/// advertises it: its ``isa`` string resolved, or everything the simulator
/// has.
///
/// Raises ``ValueError`` for an ISA string the simulator cannot implement.
///
/// # Errors
///
/// A ``ValueError`` when the dict is not a configuration or names an ISA
/// the simulator cannot implement.
#[pyfunction]
pub fn isa_string(py: Python<'_>, config_dict: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(config_from_py(py, config_dict)?.isa_string())
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
    config_to_py_dict(py, &*config_from_py(py, config_dict)?)
}
