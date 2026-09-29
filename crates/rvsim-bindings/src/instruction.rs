//! Instruction Python binding.
//!
//! A single committed instruction returned by `Cpu.step()`.

use pyo3::prelude::*;

/// A single committed instruction from the pipeline.
#[pyclass(name = "Instruction")]
#[derive(Clone)]
pub struct PyInstruction {
    #[pyo3(get)]
    pub pc: u64,
    #[pyo3(get)]
    pub raw: u32,
    #[pyo3(get)]
    pub asm: String,
    #[pyo3(get)]
    pub cycles: u64,
}

#[pymethods]
impl PyInstruction {
    fn __repr__(&self) -> String {
        format!("Instruction(pc={:#010x}, asm={:?}, cycles={})", self.pc, self.asm, self.cycles)
    }
}
