//! Simulator-level error types.
//!
//! This module defines [`SimError`], the top-level error type returned from
//! public simulator APIs. All variants include human-readable messages with
//! actionable context so users can diagnose failures without reading source code.

use thiserror::Error;

use crate::common::VirtAddr;

/// Errors that can be returned from the simulator's public API.
///
/// Each variant is designed to give the user enough context to understand
/// what went wrong and, where possible, how to fix it.
#[derive(Debug, Error)]
pub enum SimError {
    /// A binary or firmware file could not be read from disk.
    ///
    /// Check that the path exists and that the process has read permission.
    #[error("could not read file '{path}': {source}")]
    FileRead {
        /// Path that was attempted.
        path: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The directory the guest's writefile pseudo-operation writes into
    /// could not be created.
    #[error("could not create the output directory '{path}': {source}")]
    OutputDir {
        /// The directory.
        path: String,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The connection to a debugger failed.
    #[error("debugger connection: {source}")]
    Debugger {
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The bytes given as a program are not an ELF image.
    #[error("not an ELF image")]
    NotAnElf,
    /// A program SE mode cannot run.
    #[error("cannot run this program in SE mode: {reason}")]
    UnsupportedProgram {
        /// What about it.
        reason: String,
    },
    /// The host could not map the configured RAM.
    #[error("could not map {bytes} bytes of RAM: {source}")]
    RamAllocation {
        /// The RAM size asked for.
        bytes: u64,
        /// The host's error.
        source: std::io::Error,
    },
    /// RAM could not hold the process.
    #[error("the program's memory could not be set up: {0:?}")]
    ProcessMemory(crate::system::process::SpaceError),

    /// A kernel panic was detected via the `tohost`/panic sentinel mechanism.
    ///
    /// The guest OS crashed. Inspect the serial output for the panic message.
    #[error("kernel panic detected at cycle {cycle}; check serial output for details")]
    KernelPanic {
        /// Simulator cycle at which the panic was detected.
        cycle: u64,
    },

    /// A hart outside WFI retired no instruction for `cycles` cycles. Its
    /// pipeline is stuck, which is a simulator bug, unless the guest keeps
    /// taking traps without ever retiring an instruction.
    #[error("hart {hart} retired nothing for {cycles} cycles at PC {pc:#x}: its pipeline is stuck")]
    Stalled {
        /// The hart.
        hart: usize,
        /// Its architectural PC: the instruction that did not retire.
        pc: VirtAddr,
        /// Cycles since it last retired an instruction.
        cycles: u64,
    },

    /// With the cache audit on, an event left a cache invariant broken.
    #[error("cache invariant broken at cycle {cycle}: {violation}")]
    CacheInvariant {
        /// Simulator cycle of the event.
        cycle: u64,
        /// The invariant and the line it broke for.
        violation: crate::system::coherence_audit::Violation,
    },
}
