//! The simulation kernel every timed component is built on.
//!
//! The event queue and the `Handle` components receive events through, the
//! packets they exchange, the identifiers that address them, statistics,
//! and tracing.

pub mod components;
pub mod events;
pub mod handle;
pub mod packet;
pub mod stats;
pub mod trace;
