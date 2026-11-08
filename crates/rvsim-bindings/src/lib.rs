//! Python bindings for the RISC-V system simulator (`PyO3`).

#![expect(
    unnameable_types,
    reason = "pyo3 implements its public traits for every #[pyclass], which only Python names"
)]

use pyo3::prelude::*;

/// The core's default and minimal machines and its configuration check.
mod config;
/// Conversion between Python config dicts and the core's `Config`.
mod conversion;
/// The exceptions the simulator raises.
mod errors;
/// Instruction binding (`PyInstruction` exposed as `Instruction`).
mod instruction;
/// The Python package's enums the extension returns.
mod python_enums;
/// Simulator binding (`PySimulator` exposed as `Simulator`).
mod simulator;
/// Pipeline snapshot binding (`PyPipelineSnapshot` exposed as `PipelineSnapshot`).
mod snapshot;
/// Statistics binding (`PyStats` exposed as `Stats`, and query results).
mod stats;
/// Register, CSR, and memory view bindings.
mod views;

#[cfg(test)]
mod tests;

/// The version of the extension, as built.
#[pyfunction]
#[must_use]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Registers all public classes and functions onto the Python module.
///
/// # Errors
///
/// Returns the error Python raised while adding a class or function.
fn register_emulator_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    errors::register(m)?;
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
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(instruction::disassemble, m)?)?;
    m.add_function(wrap_pyfunction!(instruction::csr_names, m)?)?;
    m.add_function(wrap_pyfunction!(instruction::register_names, m)?)?;
    m.add_function(wrap_pyfunction!(config::isa_string, m)?)?;
    m.add_function(wrap_pyfunction!(config::default_config, m)?)?;
    m.add_function(wrap_pyfunction!(config::minimal_config, m)?)?;
    m.add_function(wrap_pyfunction!(config::check_config, m)?)?;
    m.add_function(wrap_pyfunction!(config::component_defaults, m)?)?;

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
