//! RISC-V system simulator library.
//!
//! This crate implements a cycle-accurate RISC-V RV64GC simulator with the following:
//! 1. **Core:** 10-stage pipeline (Fetch1/2, Decode, Rename, Issue, Execute, Mem1/2, WB, Commit),
//!    GPR/FPR, and CSR state.
//! 2. **Memory:** MMU, TLB, caches, prefetchers, and configurable memory controllers.
//! 3. **ISA:** Decoding and execution for RV64I/M/A/F/D/C and privileged operations.
//! 4. **SoC:** Interconnect, RAM, and MMIO devices (UART, CLINT, PLIC, VirtIO, etc.).
//! 5. **Simulation:** `Simulator` (owns CPU + pipeline), loader, configuration, and statistics.

/// Common types and constants (addresses, registers, traps, access types).
pub mod common;
/// Simulator configuration (defaults, enums, hierarchical config structures).
pub mod config;
/// CPU core (arch state, execution helpers, memory, trap) and pipeline.
pub mod core;
/// Instruction set (decode, instruction, ABI, RV64I/M/A/F/D, RVC, privileged).
pub mod isa;
/// Simulation: `Simulator`, binary loader, and kernel setup.
pub mod sim;
/// System-on-chip (builder, bus, devices, memory, traits).
pub mod soc;
/// Simulation statistics collection and reporting.
pub mod stats;

/// Root configuration type; use `Config::default()` or deserialize from Python/JSON.
pub use crate::config::Config;
/// Main CPU type; holds caches, MMU, and stats.
pub use crate::core::Cpu;
/// Top-level simulator; owns the CPU and pipeline side-by-side.
pub use crate::sim::simulator::Simulator;
/// Top-level system (bus, memory controller, devices); construct with `System::new`.
pub use crate::soc::System;
