//! Instruction pipelines.
//!
//! A shared front end (fetch, decode, rename) feeds either the in-order or
//! the out-of-order back end through inter-stage latches. The reorder
//! buffer, rename structures and load/store queues are shared by both.

/// Backend pipeline stages.
pub mod backend;

/// Execution engine traits and pipeline dispatch.
pub mod engine;

/// Where in the pipeline an exception was first detected.
pub mod exception;

/// Frontend pipeline stages.
pub mod frontend;

/// Inter-stage pipeline latches.
pub mod latches;

/// Load queue, store buffers and write-combining buffer.
pub mod lsq;

/// Mailbox-drain logic.
///
/// Wakes the operations parked on arriving `MemResp` packets.
pub mod mailbox;

/// In-flight memory requests the pipeline is waiting on.
///
/// Fetches, loads, stores and page-table walks, keyed by `ReqId`.
pub mod outstanding;

/// Register renaming structures and branch checkpoints.
pub mod rename;

/// Reorder buffer for in-order commit.
pub mod rob;

/// Point-in-time pipeline state snapshot.
pub mod snapshot;

/// Squashes execute asks for and the pipeline takes after the redirect latency.
pub mod squash;
