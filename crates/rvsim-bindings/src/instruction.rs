//! Instruction Python binding.
//!
//! A single committed instruction returned by `Cpu.step()`.

use pyo3::prelude::*;

/// A single committed instruction from the pipeline.
#[pyclass(name = "Instruction")]
#[derive(Clone, Debug)]
pub struct PyInstruction {
    /// The address it retired from.
    #[pyo3(get)]
    pub(crate) pc: u64,
    /// Its encoding.
    #[pyo3(get)]
    pub(crate) raw: u32,
    /// Its disassembly.
    #[pyo3(get)]
    pub(crate) asm: String,
    /// Cycles `step` ran to retire it.
    #[pyo3(get)]
    pub(crate) cycles: u64,
}

#[pymethods]
impl PyInstruction {
    fn __repr__(&self) -> String {
        format!("Instruction(pc={:#010x}, asm={:?}, cycles={})", self.pc, self.asm, self.cycles)
    }
}
