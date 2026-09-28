//! # Simulation Unit Tests
//!
//! This module contains unit tests for simulation-related functionality,
//! including binary loading and system initialization.

/// Tests for the checks applied to a configuration before use.
pub mod config_validation;

/// Tests for the generated device tree.
pub mod dtb;

/// Tests for RAM writes that bypass the harts' store paths.
pub mod external_writes;

/// Tests for accesses taking effect against the memory image.
pub mod global_memory;

/// Tests for the main execution loop and pipeline coordination.
pub mod execution;

/// Tests for binary loader and kernel setup.
pub mod loader;

/// Tests for the hierarchical stats query language.
pub mod stats_query;
