//! Simulator-level error types.
//!
//! This module defines [`SimError`], the top-level error type returned from
//! public simulator APIs. All variants include human-readable messages with
//! actionable context so users can diagnose failures without reading source code.

use thiserror::Error;

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

    /// The simulator appears to be stuck at the same program counter for an
    /// unusually large number of cycles.
    ///
    /// This is not necessarily fatal — it may be a legitimate WFI spin-wait —
    /// but it is surfaced so callers can decide whether to abort.
    #[error(
        "potential hang: PC {pc:#x} unchanged for {cycle_count} cycles \
         (instruction word: {inst:#010x})"
    )]
    HangDetected {
        /// Program counter that has not changed.
        pc: u64,
        /// Raw instruction word at that address (0 if unreadable).
        inst: u32,
        /// Number of consecutive cycles spent at this PC.
        cycle_count: u64,
    },

    /// A kernel panic was detected via the `tohost`/panic sentinel mechanism.
    ///
    /// The guest OS crashed. Inspect the serial output for the panic message.
    #[error("kernel panic detected at cycle {cycle}; check serial output for details")]
    KernelPanic {
        /// Simulator cycle at which the panic was detected.
        cycle: u64,
    },
}
