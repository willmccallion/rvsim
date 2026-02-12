//! System (SoC) Python binding.
//!
//! Exposes the top-level `System` to Python: create from config dict and optional disk path,
//! load binaries at address, then pass the system to `PyCpu` (consuming the reference).

use crate::conversion::py_dict_to_config;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use riscv_core::config::Config;
use riscv_core::soc::System;

/// Python-exposed system: wraps the core `System` (bus, memory controller, devices). Consumed by `PyCpu::new`.
#[pyclass]
pub struct PySystem {
    pub inner: Option<System>,
}

#[pymethods]
impl PySystem {
    /// Creates a new system from a config dict and optional disk image path.
    ///
    /// After creation, call `load_binary` if needed, then pass this object to `PyCpu::new`;
    /// the system is moved into the CPU and cannot be used again.
    #[new]
    #[pyo3(signature = (config_dict, disk_path=None))]
    fn new(
        py: Python,
        config_dict: &Bound<'_, PyAny>,
        disk_path: Option<String>,
    ) -> PyResult<Self> {
        let config: Config = py_dict_to_config(py, config_dict)?;
        let disk = disk_path.unwrap_or_default();

        let system = System::new(&config, &disk);

        Ok(PySystem {
            inner: Some(system),
        })
    }

    /// Loads a byte sequence into system memory at the given physical address.
    ///
    /// # Arguments
    ///
    /// * `data` - Bytes to write.
    /// * `addr` - Physical base address.
    fn load_binary(&mut self, data: Vec<u8>, addr: u64) -> PyResult<()> {
        if let Some(sys) = &mut self.inner {
            sys.load_binary_at(&data, addr);
            Ok(())
        } else {
            Err(PyRuntimeError::new_err(
                "System has already been consumed by CPU",
            ))
        }
    }
}
