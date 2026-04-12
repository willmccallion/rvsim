//! Pipeline latch structures for inter-stage communication.
//!
//! This module defines the entry types carried between the 10-stage pipeline:
//! Fetch1 → Fetch2 → Decode → Rename → Issue → Execute → Mem1 → Mem2 → Writeback → Commit.
//!
//! 1. **Instruction Flow:** Structures for carrying state between pipeline stages.
//! 2. **Superscalar Support:** Multi-entry latches for wide-issue configurations.
//! 3. **Trap Propagation:** Carrying architectural exceptions and interrupts through the pipeline.

use crate::common::error::{ExceptionStage, LrScRecord, PteUpdate, SfenceVmaInfo, Trap};
use crate::common::{InstSize, PhysAddr, RegIdx, VirtAddr};
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::rob::RobTag;
use crate::core::pipeline::signals::ControlSignals;
use crate::core::units::bru::Ghr;
use crate::core::units::vpu::types::{ElemIdx, Sew, VecPhysReg};

/// Index into the `O3Engine::vec_mem_inflight` tracking table.
/// Strongly typed to prevent confusion with other indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VecMemInflightIdx(pub usize);

/// Metadata for a vector memory element micro-op flowing through Memory1/Memory2.
#[derive(Clone, Debug)]
pub struct VecMemElement {
    /// Index into O3Engine::vec_mem_inflight for the parent instruction.
    pub parent_idx: VecMemInflightIdx,
    /// Element index within the vector register (for writeback targeting).
    pub elem_idx: ElemIdx,
    /// Effective element width for this access.
    pub eew: Sew,
    /// Destination physical vector register for this element's data.
    pub vd_phys: VecPhysReg,
    /// Whether this is a store (vs load).
    pub is_store: bool,
}

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
    pub inst_size: InstSize,
    /// Whether the branch predictor predicted this instruction as taken.
    pub pred_taken: bool,
    /// Predicted target address for branch/jump instructions.
    pub pred_target: u64,
    /// Trap that occurred during fetch, if any.
    pub trap: Option<Trap>,
    /// Pipeline stage where the exception was first detected.
    pub exception_stage: Option<ExceptionStage>,
    /// GHR snapshot captured at prediction time for speculative history repair.
    pub ghr_snapshot: Ghr,
    /// RAS pointer snapshot captured at prediction time for speculative recovery.
    pub ras_snapshot: usize,
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
    pub inst_size: InstSize,
    /// First source register index (rs1).
    pub rs1: RegIdx,
    /// Second source register index (rs2).
    pub rs2: RegIdx,
    /// Third source register index (rs3).
    pub rs3: RegIdx,
    /// Destination register index (rd).
    pub rd: RegIdx,
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
    /// GHR snapshot captured at prediction time for speculative history repair.
    pub ghr_snapshot: Ghr,
    /// RAS pointer snapshot captured at prediction time for speculative recovery.
    pub ras_snapshot: usize,
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
    pub inst_size: InstSize,
    /// Destination register index (rd).
    pub rd: RegIdx,
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
    pub inst_size: InstSize,
    /// Destination register index (rd).
    pub rd: RegIdx,
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
    pub paddr: PhysAddr,
    /// Whether the branch predictor predicted taken.
    pub pred_taken: bool,
    /// Predicted target address.
    pub pred_target: u64,
    /// Trap during fetch (alignment, TLB fault).
    pub trap: Option<Trap>,
    /// Pipeline stage where the exception was detected.
    pub exception_stage: Option<ExceptionStage>,
    /// GHR snapshot captured at prediction time for speculative history repair.
    pub ghr_snapshot: Ghr,
    /// RAS pointer snapshot captured at prediction time for speculative recovery.
    pub ras_snapshot: usize,
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
    pub inst_size: InstSize,
    /// Source register 1 index.
    pub rs1: RegIdx,
    /// Source register 2 index.
    pub rs2: RegIdx,
    /// Source register 3 index (FMA).
    pub rs3: RegIdx,
    /// Destination register index.
    pub rd: RegIdx,
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
    /// Physical register for rs1 (O3 PRF path; PhysReg(0) for in-order).
    pub rs1_phys: PhysReg,
    /// Physical register for rs2 (O3 PRF path).
    pub rs2_phys: PhysReg,
    /// Physical register for rs3 (O3 PRF path).
    pub rs3_phys: PhysReg,
    /// Physical destination register allocated at rename (O3 PRF path).
    pub rd_phys: PhysReg,
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
    /// GHR snapshot captured at prediction time for speculative history repair.
    pub ghr_snapshot: Ghr,
    /// RAS pointer snapshot captured at prediction time for speculative recovery.
    pub ras_snapshot: usize,
    /// Physical vector registers for vs1 LMUL group (O3 backend).
    pub vs1_phys: [VecPhysReg; 8],
    /// Physical vector registers for vs2 LMUL group (O3 backend).
    pub vs2_phys: [VecPhysReg; 8],
    /// Physical vector registers for vs3/vd-as-source LMUL group (O3 backend).
    pub vs3_phys: [VecPhysReg; 8],
    /// Physical vector registers for vd destination LMUL group (O3 backend).
    pub vd_phys: [VecPhysReg; 8],
    /// Number of registers in vs1 LMUL group.
    pub vec_src1_count: u8,
    /// Number of registers in vs2 LMUL group.
    pub vec_src2_count: u8,
    /// Number of registers in vs3 LMUL group.
    pub vec_src3_count: u8,
    /// Physical register for v0 mask (populated by rename for masked vector ops).
    pub mask_phys: VecPhysReg,
}

/// Entry from Execute -> Memory1 latch.
#[derive(Clone, Debug)]
pub struct ExMem1Entry {
    /// ROB tag.
    pub rob_tag: RobTag,
    /// Program counter.
    pub pc: u64,
    /// Raw instruction.
    pub inst: u32,
    /// Instruction size.
    pub inst_size: InstSize,
    /// Destination register.
    pub rd: RegIdx,
    /// Physical destination register (O3 PRF path).
    pub rd_phys: PhysReg,
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
    /// FP exception flags from this instruction (deferred to commit).
    pub fp_flags: u8,
    /// Deferred SFENCE.VMA operands for commit-time TLB invalidation.
    pub sfence_vma: Option<SfenceVmaInfo>,
    /// Vector memory element metadata (None for scalar ops).
    pub vec_mem: Option<VecMemElement>,
}

impl Default for ExMem1Entry {
    fn default() -> Self {
        Self {
            rob_tag: RobTag::default(),
            pc: 0,
            inst: 0,
            inst_size: InstSize::default(),
            rd: RegIdx::default(),
            rd_phys: PhysReg::default(),
            alu: 0,
            store_data: 0,
            ctrl: ControlSignals::default(),
            trap: None,
            exception_stage: None,
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        }
    }
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
    pub inst_size: InstSize,
    /// Destination register.
    pub rd: RegIdx,
    /// Physical destination register (O3 PRF path).
    pub rd_phys: PhysReg,
    /// ALU result (original, for non-memory ops).
    pub alu: u64,
    /// Virtual address.
    pub vaddr: VirtAddr,
    /// Physical address after translation.
    pub paddr: PhysAddr,
    /// Store data.
    pub store_data: u64,
    /// Control signals.
    pub ctrl: ControlSignals,
    /// Trap from memory1 (translation fault).
    pub trap: Option<Trap>,
    /// Exception stage.
    pub exception_stage: Option<ExceptionStage>,
    /// FP exception flags from this instruction (deferred to commit).
    pub fp_flags: u8,
    /// Cycle at which this entry's memory operation completes (O3 per-op latency).
    /// For non-memory ops or in-order backend, defaults to 0 (ready immediately).
    pub complete_cycle: u64,
    /// Deferred PTE A/D bit update from address translation (applied at commit).
    pub pte_update: Option<PteUpdate>,
    /// Deferred SFENCE.VMA operands for commit-time TLB invalidation.
    pub sfence_vma: Option<SfenceVmaInfo>,
    /// Vector memory element metadata (flows through from ExMem1Entry).
    pub vec_mem: Option<VecMemElement>,
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
    pub inst_size: InstSize,
    /// Destination register.
    pub rd: RegIdx,
    /// Physical destination register (O3 PRF path).
    pub rd_phys: PhysReg,
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
    /// FP exception flags from this instruction (deferred to commit).
    pub fp_flags: u8,
    /// Deferred PTE A/D bit update from address translation (applied at commit).
    pub pte_update: Option<PteUpdate>,
    /// Deferred SFENCE.VMA operands for commit-time TLB invalidation.
    pub sfence_vma: Option<SfenceVmaInfo>,
    /// Deferred LR/SC reservation action for commit-time application.
    pub lr_sc: Option<LrScRecord>,
    /// Vector memory element metadata (flows through from ExMem1Entry).
    pub vec_mem: Option<VecMemElement>,
}
