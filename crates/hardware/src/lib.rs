//! Cycle-accurate RV64GC + RVV 1.0 system simulator.
//!
//! The modules are layered; each depends only on those listed before it,
//! except that the pipelines in `uarch` run on the per-core views `system`
//! defines.

/// Addresses, identifiers, access kinds and tracing shared by every layer.
pub mod common;

/// What the RISC-V ISA defines: encodings, CSRs, fields and vocabulary.
pub mod isa;

/// Simulator configuration.
pub mod config;

/// Architectural state: harts, registers, CSRs, traps, PMP.
pub mod arch;

/// Instruction semantics shared by every engine: decode, execute, retire.
pub mod exec;

/// Simulation kernel: events, packets, component ids, memory image, stats.
pub mod sim;

/// System-on-chip: bus, coherence, devices and memory controllers.
pub mod soc;

/// Microarchitecture: the timing model of a core.
pub mod uarch;

/// The whole simulated system: `Simulator`, its state, loading, checkpoints.
pub mod system;

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
/// Top-level simulator; owns the `SystemState` and pipeline side-by-side.
pub use crate::system::simulator::Simulator;
/// Simulator-side architectural state: hart, core, bus, caches, MMU, stats.
pub use crate::system::{CoreCtx, StageCtx, SystemState, Uncore};
