//! Python bindings for the RISC-V system simulator.
//!
//! This crate exposes the simulator to Python via PyO3. It provides:
//! 1. **CPU:** `Cpu` — the sole public entry point for simulation.
//! 2. **Views:** `Instruction`, `Registers`, `Csrs`, `Memory` for CPU introspection.
//! 3. **Utilities:** `version()` and `disassemble()`.

use pyo3::prelude::*;

/// Python dict to Rust `Config` conversion.
pub mod conversion;
/// CPU binding (`PyCpu` exposed as `Cpu`).
pub mod cpu;
/// Instruction binding (`PyInstruction` exposed as `Instruction`).
pub mod instruction;
/// Pipeline snapshot binding (`PyPipelineSnapshot` exposed as `PipelineSnapshot`).
pub mod snapshot;
/// Statistics (internal, not exposed to Python).
pub mod stats;
/// Utility functions (e.g., version).
pub mod utils;
/// Register, CSR, and memory view bindings.
pub mod views;

/// Registers all public classes and functions onto the Python module.
pub fn register_emulator_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<cpu::PyCpu>()?;

    m.add_class::<instruction::PyInstruction>()?;
    m.add_class::<snapshot::PyPipelineSnapshot>()?;
    m.add_class::<views::Registers>()?;
    m.add_class::<views::Csrs>()?;
    m.add_class::<views::Memory>()?;

    m.add_function(wrap_pyfunction!(utils::version, m)?)?;
    m.add_function(wrap_pyfunction!(utils::disassemble, m)?)?;

    Ok(())
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_emulator_module(m)?;
    Ok(())
}
