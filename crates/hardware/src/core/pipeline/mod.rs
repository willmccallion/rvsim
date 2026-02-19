//! Instruction pipeline implementation.
//!
//! This module contains the 10-stage pipeline infrastructure including:
//! 1. **Engine:** Traits for pluggable backends (in-order, out-of-order).
//! 2. **ROB:** Reorder buffer for in-order commit.
//! 3. **Store Buffer:** Deferred store writes with forwarding.
//! 4. **Frontend:** Fetch1, Fetch2, Decode, and Rename stages (shared across backends).
//! 5. **Backend:** Issue, Execute, Memory1, Memory2, Writeback, and Commit stages.
//! 6. **Latches:** Inter-stage buffers for communication between pipeline stages.
//! 7. **Signals:** Control signals generated during instruction decoding.

/// Execution engine traits and pipeline dispatch.
pub mod engine;

/// Inter-stage pipeline latches.
pub mod latches;

/// Reorder buffer for in-order commit.
pub mod rob;

/// Control signals generated during instruction decode.
pub mod signals;

/// Tag-based register scoreboard.
pub mod scoreboard;

/// Store buffer with forwarding.
pub mod store_buffer;

/// Traits for pipeline stage components.
pub mod traits;

/// Frontend pipeline stages.
pub mod frontend;

/// Backend pipeline stages.
pub mod backend;
