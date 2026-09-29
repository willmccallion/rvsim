//! The rvsim-core test suite.
//!
//! `unit` mirrors the crate's modules, `integration` runs programs through
//! whole pipelines and systems, `fuzz` holds property tests, and `support`
//! the builders, harness and mocks they share.

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

pub mod fuzz;
pub mod integration;
pub mod support;
pub mod unit;
