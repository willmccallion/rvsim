//! # Simulation Unit Tests
//!
//! This module contains unit tests for simulation-related functionality,
//! including binary loading and system initialization.

/// Tests for the generated device tree.
pub mod dtb;

/// Tests for the main execution loop and pipeline coordination.
pub mod execution;

/// Tests for binary loader and kernel setup.
pub mod loader;

/// Tests for the hierarchical stats query language.
pub mod stats_query;
