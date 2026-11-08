//! The rvsim-core test suite.
//!
//! `unit` mirrors the crate's modules, `integration` runs programs through
//! whole pipelines and systems, `fuzz` holds property tests, and `support`
//! the builders, harness and mocks they share. Every suite drives the
//! model through its internals, so the tests live in the crate.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    reason = "tests build bit patterns and expected values directly"
)]

pub mod fuzz;
pub mod integration;
pub mod support;
pub mod unit;
