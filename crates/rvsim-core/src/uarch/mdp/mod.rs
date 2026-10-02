//! Memory dependence prediction (MDP) implementations.
//!
//! This module provides a gem5-style [`MemDepUnit`] that owns a predictor and
//! maintains per-instruction dependency records with explicit wakeup. The
//! pipeline queries the unit at dispatch to get a cached [`MemDepState`] for
//! each instruction, and notifies it when stores resolve so that waiting
//! instructions can be woken.

mod predictor;
mod store_set;
mod unit;

pub use unit::{MemDepState, MemDepUnit};
