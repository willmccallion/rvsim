//! Python bindings for the RISC-V system simulator.
//!
//! This crate exposes the simulator to Python via PyO3. It provides:
//! 1. **System and CPU:** `PySystem` and `PyCpu` for configuration and cycle stepping.
//! 2. **Statistics:** `PyStats` for performance metrics and selective section printing.
//! 3. **Memory and devices:** `PyMemory`, `PyUart`, `PyPlic`, `PyVirtioBlock` for introspection.
//! 4. **Utilities:** Version string and conversion helpers for Python↔Rust types.

use pyo3::prelude::*;

/// Python dict to Rust `Config` conversion.
pub mod conversion;
/// CPU binding (`PyCpu`).
pub mod cpu;
/// Device bindings (UART, PLIC, VirtIO).
pub mod devices;
/// Memory binding (`PyMemory`).
pub mod memory;
/// Statistics binding (`PyStats`).
pub mod stats;
/// System binding (`PySystem`).
pub mod system;
/// Utility functions (e.g., version).
pub mod utils;

/// Registers all emulator classes and functions onto the given Python module.
///
/// Called from the `#[pymodule]` entry point to expose `PyCpu`, `PySystem`, `PyStats`,
/// `PyMemory`, device types, and `version`.
///
/// # Arguments
///
/// * `m` - The Python module to register types and functions on.
///
/// # Returns
///
/// `Ok(())` on success, or a `PyErr` if registration fails.
pub fn register_emulator_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<cpu::PyCpu>()?;
    m.add_class::<system::PySystem>()?;
    m.add_class::<stats::PyStats>()?;
    m.add_class::<memory::PyMemory>()?;

    m.add_class::<devices::uart::PyUart>()?;
    m.add_class::<devices::plic::PyPlic>()?;
    m.add_class::<devices::virtio::PyVirtioBlock>()?;

    m.add_function(wrap_pyfunction!(utils::version, m)?)?;

    Ok(())
}

#[pymodule]
fn riscv_emulator(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_emulator_module(m)?;
    Ok(())
}
