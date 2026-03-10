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

/// CSR address newtype (12-bit, 0x000–0xFFF).
pub mod csr_addr;

/// Instruction size enum (2 for compressed, 4 for standard).
pub mod inst_size;

/// Memory access type definitions.
pub mod data;

/// Error types and trap definitions.
pub mod error;

/// Architectural register index newtype (5-bit, 0–31).
pub mod reg_idx;

/// Register file implementation.
pub mod reg;

/// Top-level simulator error type.
pub mod sim_error;

pub use addr::{Asid, IrqId, PhysAddr, Ppn, VirtAddr, Vpn};
pub use constants::{PAGE_SHIFT, VPN_MASK};
pub use csr_addr::CsrAddr;
pub use data::AccessType;
pub use error::{ExceptionStage, LrScRecord, PteUpdate, SfenceVmaInfo, TranslationResult, Trap};
pub use inst_size::InstSize;
pub use reg::RegisterFile;
pub use reg_idx::RegIdx;
pub use sim_error::SimError;
