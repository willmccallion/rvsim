//! PLIC (Platform-Level Interrupt Controller) Python binding.
//!
//! Exposes a placeholder type for PLIC device introspection from Python; actual IRQ state is managed by the core simulator.

use pyo3::prelude::*;

/// Placeholder type for PLIC device access from Python.
#[pyclass]
pub struct PyPlic {}

#[pymethods]
impl PyPlic {
    #[new]
    fn new() -> Self {
        PyPlic {}
    }
}
