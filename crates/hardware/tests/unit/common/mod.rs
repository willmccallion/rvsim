//! Common core component tests.
//!
//! This module contains unit tests for fundamental RISC-V data structures and
//! components, such as address types and register files.

/// Unit tests for address arithmetic and type construction.
///
/// This module verifies the behavior of virtual and physical address types,
/// including page offset calculations and value extraction.
pub mod address_arithmetic;

/// Unit tests for RISC-V register file indexing and behavior.
///
/// This module verifies the functionality of General Purpose Registers (GPRs)
/// and Floating-Point Registers (FPRs), ensuring correct read/write operations
/// and adherence to architectural constraints like the hardwired zero in `x0`.
pub mod register_indexing;
