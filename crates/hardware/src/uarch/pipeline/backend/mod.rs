//! Backend pipeline stages.
//!
//! The backend covers: Issue -> Execute -> Memory1 -> Memory2 -> Writeback -> Commit.
//! Shared stages (Memory1, Memory2, Writeback, Commit) are common free functions.
//! Issue and Execute differ between in-order and O3 backends.

pub mod inorder;
pub mod o3;
pub mod shared;
