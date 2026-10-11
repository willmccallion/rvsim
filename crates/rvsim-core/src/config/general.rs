//! General simulator settings: tracing, console, clock.

use super::defaults;
use serde::{Deserialize, Serialize};

/// General simulation settings and options.
///
/// Contains high-level simulation configuration such as tracing,
/// initial program counter, and direct (bare-metal) execution mode.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralConfig {
    /// Enable instruction tracing to stderr and debug output (hang detection, status updates, mode switches)
    #[serde(default)]
    pub trace_instructions: bool,

    /// Initial PC value (defaults to RAM base)
    #[serde(default = "GeneralConfig::default_start_pc")]
    pub start_pc: u64,

    /// The paging mode a user program runs under in SE mode.
    #[serde(default)]
    pub page_mode: SePagingMode,

    /// The core clock, which SE mode's time syscalls divide cycles by.
    #[serde(default = "GeneralConfig::default_clock_hz")]
    pub clock_hz: u64,
}

/// The paging modes SE mode builds a user program's tables for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub enum SePagingMode {
    /// Three-level tables over 39 bits, Linux's default on riscv64.
    #[default]
    Sv39,
    /// Four levels over 48 bits.
    Sv48,
    /// Five levels over 57 bits.
    Sv57,
}

impl SePagingMode {
    /// The MMU's mode.
    #[must_use]
    pub const fn paging_mode(self) -> crate::isa::privileged::PagingMode {
        use crate::isa::privileged::PagingMode;
        match self {
            Self::Sv39 => PagingMode::Sv39,
            Self::Sv48 => PagingMode::Sv48,
            Self::Sv57 => PagingMode::Sv57,
        }
    }
}

impl GeneralConfig {
    /// Returns the default starting program counter.
    const fn default_start_pc() -> u64 {
        defaults::RAM_BASE
    }

    /// 1 GHz.
    const fn default_clock_hz() -> u64 {
        1_000_000_000
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            trace_instructions: false,
            start_pc: defaults::RAM_BASE,
            page_mode: SePagingMode::default(),
            clock_hz: Self::default_clock_hz(),
        }
    }
}

/// Where the UART console connects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
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
