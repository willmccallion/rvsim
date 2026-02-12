//! UART device Python binding.
//!
//! Exposes a placeholder type for UART device introspection from Python; actual I/O is handled by the core simulator.

use pyo3::prelude::*;

/// Placeholder type for UART device access from Python.
#[pyclass]
pub struct PyUart {}

#[pymethods]
impl PyUart {
    #[new]
    fn new() -> Self {
        PyUart {}
    }
}
