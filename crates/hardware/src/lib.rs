//! Cycle-accurate RV64GC + RVV 1.0 simulator core.

/// Common types and constants (addresses, registers, traps, access types).
/// Cache coherence: protocol, home agent, interconnect, fabric.
pub mod coherence;

/// Architectural state: harts, registers, CSRs, traps, PMP.
pub mod arch;

pub mod common;
/// Simulator configuration (defaults, enums, hierarchical config structures).
pub mod config;
/// CPU core (arch state, execution helpers, memory, trap) and pipeline.
pub mod core;
/// Instruction semantics shared by every engine: decode, execute, retire.
pub mod exec;
/// Instruction set (decode, instruction, ABI, RV64I/M/A/F/D, RVC, privileged).
pub mod isa;
/// Simulation: `Simulator`, binary loader, and kernel setup.
pub mod sim;
/// System-on-chip (builder, bus, devices, memory, traits).
pub mod soc;
/// Compile-time–gated tracing macros for every pipeline subsystem.
pub mod trace;

/// Address Space Identifier (ASID) from SATP[59:44]; prevents mixing with raw `u16` values.
pub use crate::common::Asid;
/// Interrupt Request Identifier for PLIC lines; prevents mixing with arbitrary `u32` values.
pub use crate::common::IrqId;
/// Simulator-level error type; returned by `Simulator::tick` and the binary loader.
pub use crate::common::SimError;
/// Root configuration type; use `Config::default()` or deserialize from Python/JSON.
pub use crate::config::Config;
/// 12-bit CSR address newtype; prevents mixing raw `u32` constants with address values.
pub use crate::isa::csr::CsrAddr;
/// 5-bit architectural register index (0–31); prevents mixing with arbitrary `usize` values.
pub use crate::isa::reg::RegIdx;
/// Top-level simulator; owns the `SimState` and pipeline side-by-side.
pub use crate::sim::simulator::Simulator;
/// Simulator-side architectural state: hart, core, bus, caches, MMU, stats.
pub use crate::sim::{CoreCtx, SharedState, SimState, StageCtx};
