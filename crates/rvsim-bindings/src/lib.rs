//! Python bindings for the RISC-V system simulator (`PyO3`).

use pyo3::prelude::*;

/// Python dict to Rust `Config` conversion.
pub mod conversion;
/// Instruction binding (`PyInstruction` exposed as `Instruction`).
pub mod instruction;
/// Simulator binding (`PySimulator` exposed as `Simulator`).
pub mod simulator;
/// Pipeline snapshot binding (`PyPipelineSnapshot` exposed as `PipelineSnapshot`).
pub mod snapshot;
/// Statistics (internal, not exposed to Python).
pub mod stats;
/// Utility functions (e.g., version).
pub mod utils;
/// Register, CSR, and memory view bindings.
pub mod views;

/// Registers all public classes and functions onto the Python module.
///
/// # Errors
///
/// Returns the error Python raised while adding a class or function.
pub fn register_emulator_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<simulator::PySimulator>()?;

    m.add_class::<instruction::PyInstruction>()?;
    m.add_class::<snapshot::PyPipelineSnapshot>()?;
    m.add_class::<stats::PyStats>()?;
    m.add_class::<stats::PyQueryResult>()?;
    m.add_class::<views::Registers>()?;
    m.add_class::<views::Csrs>()?;
    m.add_class::<views::Hart>()?;
    m.add_class::<views::Harts>()?;
    m.add_class::<views::Memory>()?;
    m.add_class::<views::VirtualMemory>()?;

    m.add("CHECKPOINT_VERSION", rvsim_core::system::checkpoint::VERSION)?;
    m.add_function(wrap_pyfunction!(utils::version, m)?)?;
    m.add_function(wrap_pyfunction!(utils::disassemble, m)?)?;

    Ok(())
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Initialize tracing subscriber if RUST_LOG is set (for trace-* features).
    // Uses env-filter: RUST_LOG=rvsim::fwd=trace,rvsim::mem=trace
    use tracing_subscriber::EnvFilter;
    let mut filter = EnvFilter::from_default_env();
    if std::env::var_os("RUST_LOG").is_some_and(|v| !v.is_empty())
        && let Ok(hart_span) = "rvsim::hart=trace".parse()
    {
        filter = filter.add_directive(hart_span);
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .try_init();

    register_emulator_module(m)?;
    Ok(())
}
