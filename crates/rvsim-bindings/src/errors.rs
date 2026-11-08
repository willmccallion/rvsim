//! The exceptions the simulator raises in Python, one class per kind of
//! failure, all under `SimulationError` but a bad config.

use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use rvsim_core::common::SimError;
use rvsim_core::system::checkpoint::CheckpointError as CoreCheckpointError;

create_exception!(
    rvsim._core,
    SimulationError,
    PyRuntimeError,
    "A simulation failed. The base of every error the simulator raises but a bad config."
);
create_exception!(
    rvsim._core,
    LoadError,
    SimulationError,
    "A program could not be loaded: unreadable, not an ELF, or not runnable in SE mode."
);
create_exception!(
    rvsim._core,
    CheckpointError,
    SimulationError,
    "A checkpoint could not be saved or restored into this simulator."
);
create_exception!(
    rvsim._core,
    KernelPanic,
    SimulationError,
    "The guest kernel panicked; its console output says why."
);
create_exception!(
    rvsim._core,
    Stalled,
    SimulationError,
    "A hart retired nothing for the stall limit's cycles: its pipeline is stuck."
);
create_exception!(
    rvsim._core,
    ConfigError,
    PyValueError,
    "A config is not one the simulator accepts, or describes a machine it cannot build."
);

/// The Python exception for `error`.
pub fn sim_error(error: &SimError) -> PyErr {
    let message = error.to_string();
    match error {
        SimError::FileRead { .. }
        | SimError::NotAnElf
        | SimError::UnsupportedProgram { .. }
        | SimError::ProcessMemory(_) => LoadError::new_err(message),
        SimError::KernelPanic { .. } => KernelPanic::new_err(message),
        SimError::Stalled { .. } => Stalled::new_err(message),
        SimError::OutputDir { .. }
        | SimError::Debugger { .. }
        | SimError::RamAllocation { .. }
        | SimError::TooManyUpstreams { .. }
        | SimError::CacheInvariant { .. } => SimulationError::new_err(message),
    }
}

/// The Python exception for a checkpoint that failed to save or restore.
pub fn checkpoint_error(error: &CoreCheckpointError) -> PyErr {
    CheckpointError::new_err(error.to_string())
}

/// Adds the exception classes to the extension module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("SimulationError", py.get_type::<SimulationError>())?;
    m.add("LoadError", py.get_type::<LoadError>())?;
    m.add("CheckpointError", py.get_type::<CheckpointError>())?;
    m.add("KernelPanic", py.get_type::<KernelPanic>())?;
    m.add("Stalled", py.get_type::<Stalled>())?;
    m.add("ConfigError", py.get_type::<ConfigError>())
}
