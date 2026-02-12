//! # Hardware Testing Library
//!
//! This module serves as the central entry point for the hardware testing suite.
//! It organizes various testing methodologies, including unit tests and shared
//! utilities, while providing a structure for integration, fuzzing, and
//! compliance tests.

/// Shared test infrastructure for hardware simulation tests.
///
/// This module provides a suite of utilities to simplify writing hardware-level tests,
/// including:
/// - **Builders**: Fluent APIs for constructing RISC-V instructions and pipeline latch entries.
/// - **Harness**: A `TestContext` that manages CPU state, memory mapping, and execution loops.
/// - **Mocks**: Mock implementations of system components like memory, buses, and interrupt controllers.
pub mod common;

/// Unit tests for the hardware components.
///
/// This module contains fine-grained tests for individual units of logic
/// within the hardware abstraction layer.
pub mod unit;

// pub mod integration;
// pub mod fuzz;
// pub mod compliance;
