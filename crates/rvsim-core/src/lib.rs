//! Cycle-level RV64GC + RVV 1.0 system simulator.
//!
//! The crate's interface is the system: build a [`Simulator`] from a
//! [`Config`], load a program or boot a kernel, run it, and read its harts,
//! memory, statistics and pipeline state back. The modules are layered;
//! each depends only on those before it:
//!
//! 1. [`common`]: addresses, identifiers, access kinds and tracing.
//! 2. [`isa`]: what the ISA defines: encodings, CSRs, fields, vocabulary.
//! 3. [`config`]: simulator configuration.
//! 4. [`arch`]: architectural state: harts, registers, CSRs, traps, PMP.
//! 5. `exec`: instruction semantics every engine shares.
//! 6. `sim`: the simulation kernel: events, packets, memory image, [`stats`].
//! 7. `soc`: the uncore: bus, caches, coherence, memory controllers, devices.
//! 8. `uarch`: the timing model of a core and the views it runs on.
//! 9. [`system`]: the whole system: `Simulator`, loading, checkpoints.
//!
//! Layers 5 to 8 are the model and stay inside the crate; the first four
//! and the last are the interface.

pub mod common;

pub mod isa;

pub mod config;

pub mod arch;

pub(crate) mod exec;

pub(crate) mod sim;

pub(crate) mod soc;

pub(crate) mod uarch;

pub mod system;

/// The statistics tree a run produces: counters, histograms and derived
/// stats keyed by path.
pub use crate::sim::stats;

#[cfg(test)]
#[allow(
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    missing_docs,
    missing_debug_implementations,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::missing_const_for_fn,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::format_collect,
    clippy::uninlined_format_args,
    clippy::float_cmp,
    clippy::single_char_pattern,
    clippy::semicolon_if_nothing_returned,
    unused_results,
    clippy::used_underscore_binding,
    clippy::unused_self,
    clippy::fn_params_excessive_bools,
    clippy::let_underscore_untyped,
    clippy::redundant_clone,
    clippy::large_types_passed_by_value
)]
mod tests;

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
/// The simulated system: build it, load it, run it, inspect it.
pub use crate::system::simulator::Simulator;
