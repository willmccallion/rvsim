//! The rvsim-core test suite.
//!
//! `unit` mirrors the crate's modules, `integration` runs programs through
//! whole pipelines and systems, `fuzz` holds property tests, and `support`
//! the builders, harness and mocks they share. Every suite drives the
//! model through its internals, so the tests live in the crate.

pub mod fuzz;
pub mod integration;
pub mod support;
pub mod unit;
