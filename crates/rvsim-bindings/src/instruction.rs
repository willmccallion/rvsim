//! Instruction Python binding.
//!
//! A single committed instruction returned by `Simulator.step()`.

use pyo3::prelude::*;

/// A single committed instruction from the pipeline.
#[pyclass(name = "Instruction", module = "rvsim._core")]
#[derive(Clone, Debug)]
pub struct PyInstruction {
    /// The address it retired from.
    #[pyo3(get)]
    pub pc: u64,
    /// Its encoding.
    #[pyo3(get)]
    pub raw: u32,
    /// Its disassembly.
    #[pyo3(get)]
    pub asm: String,
    /// Cycles `step` ran to retire it.
    #[pyo3(get)]
    pub cycles: u64,
}

#[pymethods]
impl PyInstruction {
    fn __repr__(&self) -> String {
        format!("Instruction(pc={:#010x}, asm={:?}, cycles={})", self.pc, self.asm, self.cycles)
    }
}

/// Disassemble a 32-bit RISC-V instruction encoding into a mnemonic string.
#[pyfunction]
#[must_use]
pub fn disassemble(inst: u32) -> String {
    rvsim_core::disasm::disassemble(inst)
}

/// Every CSR the simulator implements that has a name, as ``(name,
/// address)`` pairs.
#[pyfunction]
#[must_use]
pub fn csr_names() -> Vec<(&'static str, u32)> {
    rvsim_core::isa::csr::NAMED.iter().map(|&(name, addr)| (name, addr.as_u32())).collect()
}

/// The ABI names of `x0` to `x31`.
#[pyfunction]
#[must_use]
pub fn register_names() -> Vec<&'static str> {
    rvsim_core::isa::reg::RegIdx::all().map(rvsim_core::isa::reg::RegIdx::int_abi_name).collect()
}
