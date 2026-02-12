//! Pipeline latch structures for inter-stage communication.
//!
//! This module defines the buffers that connect the five stages of the pipeline. It implements:
//! 1. **Instruction Flow:** Structures for carrying state between Fetch, Decode, Execute, Memory, and Writeback.
//! 2. **Superscalar Support:** Multi-entry latches for wide-issue configurations.
//! 3. **Trap Propagation:** Carrying architectural exceptions and interrupts through the pipeline.

use crate::common::error::Trap;
use crate::core::pipeline::signals::ControlSignals;

/// Entry in the IF/ID pipeline latch (Fetch to Decode stage).
///
/// Contains instruction information fetched from memory, including the raw
/// encoding and branch prediction metadata.
#[derive(Clone, Default, Debug)]
pub struct IfIdEntry {
    /// Program counter of the instruction.
    pub pc: u64,
    /// 32-bit instruction encoding.
    pub inst: u32,
    /// Size of the instruction in bytes (2 for compressed, 4 for standard).
    pub inst_size: u64,
    /// Whether the branch predictor predicted this instruction as taken.
    pub pred_taken: bool,
    /// Predicted target address for branch/jump instructions.
    pub pred_target: u64,
    /// Trap that occurred during fetch, if any.
    pub trap: Option<Trap>,
}

/// Entry in the ID/EX pipeline latch (Decode to Execute stage).
///
/// Contains decoded instruction information, including register indices,
/// immediate values, and control signals.
#[derive(Clone, Default, Debug)]
pub struct IdExEntry {
    /// Program counter of the instruction.
    pub pc: u64,
    /// 32-bit instruction encoding.
    pub inst: u32,
    /// Size of the instruction in bytes.
    pub inst_size: u64,
    /// First source register index (rs1).
    pub rs1: usize,
    /// Second source register index (rs2).
    pub rs2: usize,
    /// Third source register index (rs3).
    pub rs3: usize,
    /// Destination register index (rd).
    pub rd: usize,
    /// Sign-extended immediate value.
    pub imm: i64,
    /// Value read from rs1 register.
    pub rv1: u64,
    /// Value read from rs2 register.
    pub rv2: u64,
    /// Value read from rs3 register.
    pub rv3: u64,
    /// Control signals for downstream pipeline stages.
    pub ctrl: ControlSignals,
    /// Trap that occurred during decode, if any.
    pub trap: Option<Trap>,
    /// Whether the branch predictor predicted this instruction as taken.
    pub pred_taken: bool,
    /// Predicted target address for branch/jump instructions.
    pub pred_target: u64,
}

/// Entry in the EX/MEM pipeline latch (Execute to Memory stage).
///
/// Contains execution results, including ALU outputs and memory operation parameters.
#[derive(Clone, Default, Debug)]
pub struct ExMemEntry {
    /// Program counter of the instruction.
    pub pc: u64,
    /// 32-bit instruction encoding.
    pub inst: u32,
    /// Size of the instruction in bytes.
    pub inst_size: u64,
    /// Destination register index (rd).
    pub rd: usize,
    /// ALU computation result or address for memory operations.
    pub alu: u64,
    /// Data to be stored (for store instructions).
    pub store_data: u64,
    /// Control signals for downstream pipeline stages.
    pub ctrl: ControlSignals,
    /// Trap that occurred during execute, if any.
    pub trap: Option<Trap>,
}

/// Entry in the MEM/WB pipeline latch (Memory to Writeback stage).
///
/// Contains memory stage results, including loaded data and final register write values.
#[derive(Clone, Default, Debug)]
pub struct MemWbEntry {
    /// Program counter of the instruction.
    pub pc: u64,
    /// 32-bit instruction encoding.
    pub inst: u32,
    /// Size of the instruction in bytes.
    pub inst_size: u64,
    /// Destination register index (rd).
    pub rd: usize,
    /// ALU computation result (for non-load instructions).
    pub alu: u64,
    /// Data loaded from memory (for load instructions).
    pub load_data: u64,
    /// Control signals for the writeback stage.
    pub ctrl: ControlSignals,
    /// Trap that occurred during memory access, if any.
    pub trap: Option<Trap>,
}

/// IF/ID pipeline latch (Fetch to Decode stage).
///
/// Supports multiple instructions per cycle for superscalar execution.
#[derive(Clone, Debug)]
pub struct IfId {
    /// Vector of fetched instruction entries.
    pub entries: Vec<IfIdEntry>,
}

impl Default for IfId {
    /// Creates an empty IF/ID latch.
    ///
    /// # Returns
    ///
    /// A new `IfId` instance with an empty entries vector.
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

/// ID/EX pipeline latch (Decode to Execute stage).
///
/// Supports multiple instructions per cycle for superscalar execution.
#[derive(Clone, Default, Debug)]
pub struct IdEx {
    /// Vector of decoded instruction entries.
    pub entries: Vec<IdExEntry>,
}

/// EX/MEM pipeline latch (Execute to Memory stage).
///
/// Supports multiple instructions per cycle for superscalar execution.
#[derive(Clone, Default, Debug)]
pub struct ExMem {
    /// Vector of execution result entries.
    pub entries: Vec<ExMemEntry>,
}

/// MEM/WB pipeline latch (Memory to Writeback stage).
///
/// Supports multiple instructions per cycle for superscalar execution.
#[derive(Clone, Default, Debug)]
pub struct MemWb {
    /// Vector of memory stage result entries.
    pub entries: Vec<MemWbEntry>,
}
