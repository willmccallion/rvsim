//! Cycle-accurate RV64GC + RVV 1.0 system simulator.
//!
//! The modules are layered; each depends only on those before it, except
//! that the pipelines in [`uarch`] run on the per-core views [`system`]
//! defines:
//!
//! 1. [`common`]: addresses, identifiers, access kinds and tracing.
//! 2. [`isa`]: what the ISA defines: encodings, CSRs, fields, vocabulary.
//! 3. [`config`]: simulator configuration.
//! 4. [`arch`]: architectural state: harts, registers, CSRs, traps, PMP.
//! 5. [`exec`]: instruction semantics every engine shares.
//! 6. [`sim`]: the simulation kernel: events, packets, memory image, stats.
//! 7. [`soc`]: the bus, coherence fabric, memory controllers and devices.
//! 8. [`uarch`]: the timing model of a core.
//! 9. [`system`]: the whole system: `Simulator`, its state, checkpoints.

pub mod common;

pub mod isa;

pub mod config;

pub mod arch;

pub mod exec;

pub mod sim;

pub mod soc;

pub mod uarch;

pub mod system;

/// Address Space Identifier (ASID) from SATP\[59:44\]; prevents mixing with raw `u16` values.
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
