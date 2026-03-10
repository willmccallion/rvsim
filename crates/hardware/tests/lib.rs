//! # Hardware Testing Library
//!
//! This module serves as the central entry point for the hardware testing suite.
//! It organizes various testing methodologies, including unit tests and shared
//! utilities, while providing a structure for integration, fuzzing, and
//! compliance tests.

// Test infrastructure and test code — relax pedantic and documentation lints.
#![allow(
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    missing_docs,
    missing_debug_implementations,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::missing_const_for_fn,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::doc_markdown,
    clippy::format_collect,
    clippy::uninlined_format_args,
    clippy::float_cmp,
    clippy::single_char_pattern,
    clippy::semicolon_if_nothing_returned,
    unused_results,
    clippy::used_underscore_binding,
    clippy::unused_self,
    clippy::fn_params_excessive_bools,
    clippy::let_underscore_untyped,
    clippy::redundant_clone
)]

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

/// Integration tests for complete instruction execution and system behavior.
pub mod integration;

// pub mod fuzz;
// pub mod compliance;
