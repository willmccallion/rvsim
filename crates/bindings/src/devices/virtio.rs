//! VirtIO block device Python binding.
//!
//! Exposes a placeholder type for VirtIO block device introspection from Python; actual disk I/O is handled by the core simulator.

use pyo3::prelude::*;

/// Placeholder type for VirtIO block device access from Python.
#[pyclass]
pub struct PyVirtioBlock {}

#[pymethods]
impl PyVirtioBlock {
    #[new]
    fn new() -> Self {
        PyVirtioBlock {}
    }
}
