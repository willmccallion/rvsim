//! Vector unit timing: lane occupancy and result chaining.

/// Result chaining between dependent vector instructions.
pub mod chaining;

/// Lane-count latency model for vector operations.
pub mod lane_model;
