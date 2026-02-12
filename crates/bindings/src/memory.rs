//! Memory introspection Python binding.
//!
//! Exposes a placeholder type for future memory introspection (e.g., RAM ranges, dump) from Python.

use pyo3::prelude::*;

/// Placeholder type for memory introspection from Python.
#[pyclass]
pub struct PyMemory {}

#[pymethods]
impl PyMemory {
    #[new]
    fn new() -> Self {
        PyMemory {}
    }
}
