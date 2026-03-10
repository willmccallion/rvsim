//! Python bindings for the RISC-V system simulator.
//!
//! This crate exposes the simulator to Python via `PyO3`. It provides:
//! 1. **CPU:** `Cpu` — the sole public entry point for simulation.
//! 2. **Views:** `Instruction`, `Registers`, `Csrs`, `Memory` for CPU introspection.
//! 3. **Utilities:** `version()` and `disassemble()`.

// PyO3 bindings — relax documentation and pedantic lints for binding-layer code.
#![allow(
    missing_docs,
    missing_debug_implementations,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::uninlined_format_args,
    clippy::format_collect,
    clippy::unused_self,
    clippy::used_underscore_binding
)]

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
    m.add_class::<views::VirtualMemory>()?;

    m.add_function(wrap_pyfunction!(utils::version, m)?)?;
    m.add_function(wrap_pyfunction!(utils::disassemble, m)?)?;

    Ok(())
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Initialize tracing subscriber if RUST_LOG is set (for trace-* features).
    // Uses env-filter: RUST_LOG=rvsim::fwd=trace,rvsim::mem=trace
    use tracing_subscriber::EnvFilter;
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .try_init();

    register_emulator_module(m)?;
    Ok(())
}
