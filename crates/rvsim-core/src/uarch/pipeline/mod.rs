//! Instruction pipelines.
//!
//! A shared front end (fetch, decode, rename) feeds either the in-order or
//! the out-of-order back end through inter-stage latches. The reorder
//! buffer, rename structures and load/store queues are shared by both.

pub mod backend;

#[cfg(feature = "commit-log")]
pub mod commit_log;

pub mod engine;

pub mod exception;

pub mod frontend;

pub mod latches;

pub mod lsq;

pub mod mailbox;

pub mod outstanding;

pub mod rename;

pub mod rob;

pub mod snapshot;

pub mod squash;
