//! Pipeline stage implementations.
//!
//! This module contains the individual implementations for the five stages of the
//! instruction pipeline. It includes:
//! 1. **Fetch:** Retrieves instructions from memory based on the PC.
//! 2. **Decode:** Decodes instructions into control signals and reads operands.
//! 3. **Execute:** Performs ALU operations and resolves branch targets.
//! 4. **Memory:** Handles data load and store operations.
//! 5. **Writeback:** Commits results to the register file and handles traps.

/// Instruction decode stage implementation.
pub mod decode;

/// Instruction execute stage implementation.
pub mod execute;

/// Instruction fetch stage implementation.
pub mod fetch;

/// Memory access stage implementation.
pub mod memory;

/// Writeback stage implementation.
pub mod writeback;

/// Decode stage entry point (ID stage).
pub use decode::decode_stage;
/// Execute stage entry point (EX stage).
pub use execute::execute_stage;
/// Fetch stage entry point (IF stage).
pub use fetch::fetch_stage;
/// Memory stage entry point (MEM stage).
pub use memory::mem_stage;
/// Writeback stage entry point (WB stage).
pub use writeback::wb_stage;
