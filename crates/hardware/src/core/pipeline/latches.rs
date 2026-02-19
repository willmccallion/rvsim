//! Pipeline latch structures for inter-stage communication.
//!
//! This module defines the entry types carried between the 10-stage pipeline:
//! Fetch1 → Fetch2 → Decode → Rename → Issue → Execute → Mem1 → Mem2 → Writeback → Commit.
//!
//! 1. **Instruction Flow:** Structures for carrying state between pipeline stages.
//! 2. **Superscalar Support:** Multi-entry latches for wide-issue configurations.
//! 3. **Trap Propagation:** Carrying architectural exceptions and interrupts through the pipeline.

use crate::common::error::{ExceptionStage, Trap};
use crate::core::pipeline::rob::RobTag;
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
    /// Pipeline stage where the exception was first detected.
    pub exception_stage: Option<ExceptionStage>,
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
    /// Pipeline stage where the exception was first detected.
    pub exception_stage: Option<ExceptionStage>,
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
    /// Pipeline stage where the exception was first detected.
    pub exception_stage: Option<ExceptionStage>,
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
    /// Pipeline stage where the exception was first detected.
    pub exception_stage: Option<ExceptionStage>,
}

// ============================================================================
// New 10-stage pipeline latch types (all carry RobTag)
// ============================================================================

/// Entry in Fetch1 -> Fetch2 latch.
///
/// Carries PC and I-TLB/branch prediction results from PC generation
/// into the I-cache access stage.
#[derive(Clone, Default, Debug)]
pub struct Fetch1Fetch2Entry {
    /// Program counter.
    pub pc: u64,
    /// Physical address after I-TLB lookup.
    pub paddr: u64,
    /// Whether the branch predictor predicted taken.
    pub pred_taken: bool,
    /// Predicted target address.
    pub pred_target: u64,
    /// Trap during fetch (alignment, TLB fault).
    pub trap: Option<Trap>,
    /// Pipeline stage where the exception was detected.
    pub exception_stage: Option<ExceptionStage>,
}

/// Entry from Rename -> Issue (also used as Issue -> Execute input).
///
/// This is the fully-decoded, register-read, ROB-tagged instruction
/// entering the backend pipeline.
#[derive(Clone, Default, Debug)]
pub struct RenameIssueEntry {
    /// ROB tag assigned during rename.
    pub rob_tag: RobTag,
    /// Program counter.
    pub pc: u64,
    /// Raw 32-bit instruction encoding.
    pub inst: u32,
    /// Instruction size in bytes.
    pub inst_size: u64,
    /// Source register 1 index.
    pub rs1: usize,
    /// Source register 2 index.
    pub rs2: usize,
    /// Source register 3 index (FMA).
    pub rs3: usize,
    /// Destination register index.
    pub rd: usize,
    /// Sign-extended immediate.
    pub imm: i64,
    /// Forwarded value for rs1.
    pub rv1: u64,
    /// Forwarded value for rs2.
    pub rv2: u64,
    /// Forwarded value for rs3.
    pub rv3: u64,
    /// Scoreboard tag for rs1 at rename time (None = read from register file).
    pub rs1_tag: Option<RobTag>,
    /// Scoreboard tag for rs2 at rename time.
    pub rs2_tag: Option<RobTag>,
    /// Scoreboard tag for rs3 at rename time.
    pub rs3_tag: Option<RobTag>,
    /// Control signals.
    pub ctrl: ControlSignals,
    /// Trap from earlier stages.
    pub trap: Option<Trap>,
    /// Exception stage.
    pub exception_stage: Option<ExceptionStage>,
    /// Branch prediction taken.
    pub pred_taken: bool,
    /// Branch prediction target.
    pub pred_target: u64,
}

/// Entry from Execute -> Memory1 latch.
#[derive(Clone, Default, Debug)]
pub struct ExMem1Entry {
    /// ROB tag.
    pub rob_tag: RobTag,
    /// Program counter.
    pub pc: u64,
    /// Raw instruction.
    pub inst: u32,
    /// Instruction size.
    pub inst_size: u64,
    /// Destination register.
    pub rd: usize,
    /// ALU result / memory virtual address.
    pub alu: u64,
    /// Store data (rs2 value).
    pub store_data: u64,
    /// Control signals.
    pub ctrl: ControlSignals,
    /// Trap from execute.
    pub trap: Option<Trap>,
    /// Exception stage.
    pub exception_stage: Option<ExceptionStage>,
}

/// Entry from Memory1 -> Memory2 latch.
#[derive(Clone, Default, Debug)]
pub struct Mem1Mem2Entry {
    /// ROB tag.
    pub rob_tag: RobTag,
    /// Program counter.
    pub pc: u64,
    /// Raw instruction.
    pub inst: u32,
    /// Instruction size.
    pub inst_size: u64,
    /// Destination register.
    pub rd: usize,
    /// ALU result (original, for non-memory ops).
    pub alu: u64,
    /// Virtual address.
    pub vaddr: u64,
    /// Physical address after translation.
    pub paddr: u64,
    /// Store data.
    pub store_data: u64,
    /// Control signals.
    pub ctrl: ControlSignals,
    /// Trap from memory1 (translation fault).
    pub trap: Option<Trap>,
    /// Exception stage.
    pub exception_stage: Option<ExceptionStage>,
}

/// Entry from Memory2 -> Writeback latch.
#[derive(Clone, Default, Debug)]
pub struct Mem2WbEntry {
    /// ROB tag.
    pub rob_tag: RobTag,
    /// Program counter.
    pub pc: u64,
    /// Raw instruction.
    pub inst: u32,
    /// Instruction size.
    pub inst_size: u64,
    /// Destination register.
    pub rd: usize,
    /// ALU result (for non-load instructions).
    pub alu: u64,
    /// Loaded data (for load instructions).
    pub load_data: u64,
    /// Control signals.
    pub ctrl: ControlSignals,
    /// Trap from memory2.
    pub trap: Option<Trap>,
    /// Exception stage.
    pub exception_stage: Option<ExceptionStage>,
}
