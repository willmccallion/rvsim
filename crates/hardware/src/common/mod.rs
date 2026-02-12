//! Common utilities and types used throughout the RISC-V system simulator.
//!
//! This module provides fundamental building blocks that are shared across all components
//! of the simulator. It includes:
//! 1. **Address Types:** Strong types for virtual and physical addresses.
//! 2. **Constants:** System-wide constants for memory, instructions, and simulation.
//! 3. **Memory Access:** Definitions for categorizing memory operations (Fetch/Read/Write).
//! 4. **Error Handling:** Trap representations and address translation result types.
//! 5. **Register Management:** A unified interface for GPR and FPR access.

/// Address type definitions (physical and virtual addresses).
pub mod addr;

/// Common constants used throughout the simulator.
pub mod constants;

/// Memory access type definitions.
pub mod data;

/// Error types and trap definitions.
pub mod error;

/// Register file implementation.
pub mod reg;

pub use addr::{PhysAddr, VirtAddr};
pub use constants::{PAGE_SHIFT, VPN_MASK};
pub use data::AccessType;
pub use error::{TranslationResult, Trap};
pub use reg::RegisterFile;
