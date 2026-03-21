//! Memory dependence prediction (MDP) implementations.
//!
//! This module provides a gem5-style [`MemDepUnit`] that owns a predictor and
//! maintains per-instruction dependency records with explicit wakeup. The
//! pipeline queries the unit at dispatch to get a cached [`MemDepState`] for
//! each instruction, and notifies it when stores resolve so that waiting
//! instructions can be woken.

pub use self::mem_dep_predictor::MdpStats;
pub use self::mem_dep_unit::{MemDepState, MemDepUnit};

/// Memory dependence predictor trait and prediction types.
mod mem_dep_predictor;

/// Blind (conservative) predictor — always waits for all older stores.
mod blind;

/// Store-set predictor (Chrysos & Emer 1998).
mod store_set;

/// Newtypes for store-set predictor internals.
mod types;

/// Gem5-style dependency tracker with wakeup.
mod mem_dep_unit;
