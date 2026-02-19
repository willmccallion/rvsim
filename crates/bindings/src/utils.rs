//! Utility functions exposed to Python.
//!
//! Provides version and other helpers for the `rvsim` module.

use pyo3::prelude::*;

/// Returns the emulator version string (e.g., for scripting or diagnostics).
///
/// # Returns
///
/// A version string such as `"0.1.0"`.
#[pyfunction]
pub fn version() -> String {
    "0.1.0".to_string()
}

/// Disassemble a 32-bit RISC-V instruction encoding into a mnemonic string.
///
/// Handles RV64GC (base integer, M, A, F, D, C extensions) and privileged
/// instructions (CSR, MRET, SRET, WFI, FENCE, etc.).
///
/// # Arguments
///
/// * `inst` - Raw 32-bit instruction word (16-bit compressed instructions
///   should be zero-extended to 32 bits before passing).
///
/// # Returns
///
/// A human-readable disassembly string such as `"addi sp, sp, -16"`,
/// or `"unknown (0x????????)"` for unrecognised encodings.
#[pyfunction]
pub fn disassemble(inst: u32) -> String {
    rvsim_core::isa::disasm::disassemble(inst)
}
