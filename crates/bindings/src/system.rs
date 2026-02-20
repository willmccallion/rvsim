//! System (SoC) Python binding.
//!
//! Exposes the top-level `System` to Python: create from config dict and optional disk path,
//! load binaries at address, then pass the system to `PyCpu` (consuming the reference).

use crate::conversion::py_dict_to_config;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use rvsim_core::config::Config;
use rvsim_core::sim::loader;
use rvsim_core::soc::System;

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

    /// Loads an ELF into system memory.
    ///
    /// Parses the ELF, loads all segments, and registers an HTIF device if a
    /// `tohost` symbol is found. Returns `(entry_point, tohost_addr)` where
    /// `tohost_addr` is `None` if not present in the ELF.
    ///
    /// Returns an error if the data is not a valid ELF.
    fn load_elf(&mut self, data: Vec<u8>) -> PyResult<(u64, Option<u64>)> {
        let sys = self
            .inner
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("System has already been consumed by CPU"))?;

        if let Some(result) = loader::try_load_elf(&data, &mut sys.bus) {
            if let Some(tohost) = result.tohost_addr {
                sys.add_htif(tohost);
            }
            Ok((result.entry, result.tohost_addr))
        } else {
            Err(PyRuntimeError::new_err(
                "Not a valid ELF file. Only ELF binaries are supported.",
            ))
        }
    }
}
