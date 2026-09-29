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

/// Where in the pipeline an exception was first detected.
pub mod exception;

/// Inter-stage pipeline latches.
pub mod latches;

/// Reorder buffer for in-order commit.
pub mod rob;

/// Tag-based register scoreboard.
pub mod scoreboard;

/// Squashes execute asks for and the pipeline takes after the redirect latency.
pub mod squash;

/// Store buffer with forwarding.
pub mod store_buffer;

/// Vector Store Buffer for in-flight vector stores.
///
/// Byte-mask forwarding and per-line drain. Parallel to `store_buffer` for scalar.
pub mod vec_store_buffer;

/// Write Combining Buffer for store coalescing.
pub mod write_buffer;

/// Physical register file with ready bits.
pub mod prf;

/// Physical register free list.
pub mod free_list;

/// Vector physical register file with ready bits and VecPrfView.
pub mod vec_prf;

/// Speculative rename map (arch reg → physical reg).
pub mod rename_map;

/// Checkpoint table for O(1) branch misprediction recovery.
pub mod checkpoint;

/// Load queue for memory ordering and violation detection.
pub mod load_queue;

/// Frontend pipeline stages.
pub mod frontend;

/// Backend pipeline stages.
pub mod backend;

/// Point-in-time pipeline state snapshot.
pub mod snapshot;

/// In-flight memory requests the pipeline is waiting on.
///
/// Covers fetches, loads, stores, and page-table walks. Entries are keyed by
/// `ReqId`; the mailbox-drain stage uses these maps to wake up parked
/// operations when their `MemResp` arrives.
pub mod outstanding;

/// Mailbox-drain logic.
///
/// Matches `MemResp` packets against the pipeline's outstanding tables and
/// wakes the parked operations.
pub mod mailbox;
