//! Memory Access Types.
//!
//! This module defines the classification of memory accesses used throughout the simulator.
//! These types are used for the following:
//! 1. **Permission Validation:** Checking Read/Write/Execute (RWX) permissions in the MMU and PMP.
//! 2. **Fault Generation:** Determining the correct page fault or access fault trap type.
//! 3. **Statistics Tracking:** Categorizing memory operations for performance analysis.

/// Type of memory access operation.
///
/// Used to distinguish between instruction fetches, data loads, and data stores
/// for proper memory management and permission enforcement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessType {
    /// Instruction fetch access.
    ///
    /// Occurs when fetching instructions from memory for the pipeline's Fetch stage.
    /// Requires Execute (X) permission.
    Fetch,

    /// Data read access.
    ///
    /// Occurs during load instructions when reading data from memory into registers.
    /// Requires Read (R) permission.
    Read,

    /// Data write access.
    ///
    /// Occurs during store instructions when writing data from registers to memory.
    /// Requires Write (W) permission.
    Write,
}
