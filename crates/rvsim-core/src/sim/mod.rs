//! The simulation kernel every timed component is built on.
//!
//! The event queue and the `Handle` components receive events through, the
//! packets they exchange, the identifiers that address them, the memory
//! image accesses perform against, and statistics.

pub mod components;
pub mod events;
pub mod handle;
pub mod memory;
pub mod packet;
pub mod stats;
