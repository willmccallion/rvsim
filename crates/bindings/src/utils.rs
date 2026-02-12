//! Utility functions exposed to Python.
//!
//! Provides version and other helpers for the `riscv_emulator` module.

use pyo3::prelude::*;

/// Returns the emulator version string (e.g., for scripting or diagnostics).
///
/// # Returns
///
/// A version string such as `"0.1.0"`.
#[pyfunction]
pub fn version() -> String {
    "0.1.0".to_string()
}
