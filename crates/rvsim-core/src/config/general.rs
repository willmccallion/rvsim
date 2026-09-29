//! General simulator settings: tracing, console, clock.

use super::defaults;
use serde::Deserialize;

/// General simulation settings and options.
///
/// Contains high-level simulation configuration such as tracing,
/// initial program counter, and direct (bare-metal) execution mode.
#[derive(Debug, Clone, Deserialize)]
pub struct GeneralConfig {
    /// Enable instruction tracing to stderr and debug output (hang detection, status updates, mode switches)
    #[serde(default)]
    pub trace_instructions: bool,

    /// Initial PC value (defaults to RAM base)
    #[serde(default = "GeneralConfig::default_start_pc")]
    pub start_pc: u64,

    /// Direct execution mode: bare-metal binary, no kernel. Traps cause exit instead of jumping to MTVEC.
    /// Default true so user only needs to change this when running a kernel.
    #[serde(default = "GeneralConfig::default_direct_mode")]
    pub direct_mode: bool,

    /// Initial stack pointer (only used when `direct_mode` is true). Defaults to `ram_base` + 16MiB if not set.
    #[serde(default)]
    pub initial_sp: Option<u64>,
}

impl GeneralConfig {
    /// Returns the default starting program counter.
    const fn default_start_pc() -> u64 {
        defaults::RAM_BASE
    }

    /// Default direct mode to true so bare-metal runs work out of the box.
    const fn default_direct_mode() -> bool {
        true
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            trace_instructions: false,
            start_pc: defaults::RAM_BASE,
            direct_mode: true,
            initial_sp: None,
        }
    }
}

/// Where the UART console connects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Console {
    /// The host's stdout, with input read from its stdin.
    #[default]
    Stdout,
    /// The host's stderr, with input read from its stdin.
    Stderr,
    /// Output discarded; no input.
    Quiet,
    /// Output kept in memory and input given by the host program, which
    /// reads and writes it through the simulator.
    Captured,
}
