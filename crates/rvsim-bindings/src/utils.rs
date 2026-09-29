//! Utility functions exposed to Python.
//!
//! Provides version and other helpers for the `rvsim` module.

use pyo3::prelude::*;

/// Returns the emulator version string.
#[pyfunction]
#[must_use]
pub fn version() -> String {
    "0.1.0".to_string()
}

/// Disassemble a 32-bit RISC-V instruction encoding into a mnemonic string.
#[pyfunction]
#[must_use]
pub fn disassemble(inst: u32) -> String {
    rvsim_core::isa::disasm::disassemble(inst)
}
