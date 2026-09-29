//! # Hart Unit Tests
//!
//! Unit tests for per-hart state manipulation, including address translation
//! and trap dispatch.

/// Tests for hart address translation.
pub mod memory;

/// Tests for trap and exception handling.
pub mod trap_handling;
