//! Utility functions exposed to Python.
//!
//! Provides version and other helpers for the `rvsim` module.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::conversion::py_dict_to_config;

/// The version of the extension, as built.
#[pyfunction]
#[must_use]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Disassemble a 32-bit RISC-V instruction encoding into a mnemonic string.
#[pyfunction]
#[must_use]
pub fn disassemble(inst: u32) -> String {
    rvsim_core::isa::disasm::disassemble(inst)
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
    let config = py_dict_to_config(py, config_dict)?;
    config.validate().map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(config.isa_string())
}
