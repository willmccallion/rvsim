//! Utility functions exposed to Python.
//!
//! Provides version and other helpers for the `rvsim` module.

use pyo3::prelude::*;

/// The version of the extension, as built.
#[pyfunction]
#[must_use]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Disassemble a 32-bit RISC-V instruction encoding into a mnemonic string.
#[pyfunction]
#[must_use]
pub fn disassemble(inst: u32) -> String {
    rvsim_core::isa::disasm::disassemble(inst)
}
