//! # Unit Components
//!
//! This module serves as the central hub for the various architectural units and
//! components of the system. It organizes the fundamental building blocks required
//! for simulation, including the processor core, ISA definitions, and SoC integration.

/// Unit tests for common RISC-V components.
///
/// This module includes tests for address arithmetic, register indexing,
/// and other shared data structures used across the emulator.
pub mod common;

/// Core definitions and fundamental logic for the unit system.
///
/// This module provides the base structures, traits, and constants that form
/// the foundation of the unit management and manipulation logic.
pub mod core;

/// Unit tests for the RISC-V Instruction Set Architecture (ISA) implementation.
///
/// This module aggregates tests for:
/// - Instruction decoding and field extraction.
/// - Disassembler mnemonic generation.
/// - Compressed (RVC) instruction expansion.
pub mod isa;

/// Unit tests for the System-on-Chip (SoC) components.
///
/// This module organizes tests for hardware devices, bus interconnects,
/// and memory controllers.
pub mod soc;

/// Unit tests for simulation statistics verification.
///
/// This module contains tests that ensure the [`SimStats`](riscv_core::stats::SimStats) structure
/// correctly tracks and calculates various performance metrics, including
/// instruction mixes, cache hit rates, and stall breakdowns.
pub mod stats_verification;
