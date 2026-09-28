//! Pipeline latch structures for inter-stage communication.
//!
//! This module defines the entry types carried between the 10-stage pipeline:
//! Fetch1 → Fetch2 → Decode → Rename → Issue → Execute → Mem1 → Mem2 → Writeback → Commit.
//!
//! 1. **Instruction Flow:** Structures for carrying state between pipeline stages.
//! 2. **Superscalar Support:** Multi-entry latches for wide-issue configurations.
//! 3. **Trap Propagation:** Carrying architectural exceptions and interrupts through the pipeline.

use crate::common::error::{DirtyUpdates, ExceptionStage, LrScRecord, SfenceVmaInfo, Trap};
use crate::common::{InstSeq, InstSize, PhysAddr, RegIdx, VirtAddr};
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::rob::RobTag;
use crate::core::pipeline::signals::ControlSignals;
use crate::core::units::vpu::mem::VecMemAddrOp;
use crate::core::units::vpu::types::{ElemIdx, Sew, VecPhysReg};
use crate::sim::state::write_log::WriteSeq;

/// A pipeline register between two stages.
///
/// It holds one bundle of entries. The producer writes only when the
/// consumer has emptied it, which is how a stall propagates backwards, and
/// the bundle becomes visible to the consumer `delay` cycles after it was
/// written: gem5's `TimeBuffer` with a depth of one bundle.
#[derive(Clone, Debug)]
pub struct Latch<T> {
    entries: Vec<T>,
    ready_at: u64,
    delay: u64,
}

impl<T> Latch<T> {
    /// An empty latch whose bundles become visible `delay` cycles after
    /// they are written.
    #[must_use]
    pub const fn new(delay: u64) -> Self {
        Self { entries: Vec::new(), ready_at: 0, delay }
    }

    /// True when no bundle is held.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries held, visible or not.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Adds `entries` to the bundle, visible from `now + delay`.
    pub fn push(&mut self, now: u64, entries: impl IntoIterator<Item = T>) {
        self.entries.extend(entries);
        self.ready_at = now.saturating_add(self.delay);
    }

    /// The bundle, if the consumer may see it at `now`.
    #[must_use]
    pub fn ready(&mut self, now: u64) -> Option<&mut Vec<T>> {
        (!self.entries.is_empty() && self.ready_at <= now).then_some(&mut self.entries)
    }

    /// Takes the bundle if the consumer may see it at `now`.
    pub fn take(&mut self, now: u64) -> Vec<T> {
        self.ready(now).map(std::mem::take).unwrap_or_default()
    }

    /// Drops the bundle.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// The entries held, for a snapshot.
    #[must_use]
    pub fn entries(&self) -> &[T] {
        &self.entries
    }
}

/// A vector memory micro-op's position among its instruction's micro-ops,
/// which tells apart the accesses of one instruction (a segment's fields
/// share their element index).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MicroOpIdx(usize);

impl MicroOpIdx {
    /// The micro-op at `index`.
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }
}

/// A vector memory micro-op flowing through Memory1/Memory2.
///
/// The parent vec mem instruction is identified by the `rob_tag` already
/// carried on the surrounding `ExMem1Entry` / `Mem1Mem2Entry` / `Mem2WbEntry`,
/// so no extra parent index is needed here.
#[derive(Clone, Debug)]
pub struct VecMemAccess {
    /// This micro-op among its instruction's micro-ops.
    pub micro_op: MicroOpIdx,
    /// Whether this is a store (vs load).
    pub is_store: bool,
    /// What the micro-op accesses.
    pub target: VecMemTarget,
}

impl VecMemAccess {
    /// Bytes the micro-op reads or writes.
    #[must_use]
    pub fn bytes(&self) -> usize {
        match &self.target {
            VecMemTarget::Element { eew, .. } => eew.bytes(),
            VecMemTarget::Span(span) => span.bytes(),
        }
    }
}

/// What a vector memory micro-op accesses.
#[derive(Clone, Debug)]
pub enum VecMemTarget {
    /// One element, or one field of a segment element.
    Element {
        /// Element index within the vector register group.
        elem_idx: ElemIdx,
        /// Effective element width for this access.
        eew: Sew,
        /// Destination physical vector register for this element's data.
        vd_phys: VecPhysReg,
    },
    /// Contiguous element accesses the vector memory datapath moves at once.
    Span(Box<VecMemSpan>),
}

/// Naturally aligned element accesses of one instruction that lie in one
/// datapath-width window, and so in one line and one page: one memory
/// access.
#[derive(Clone, Debug)]
pub struct VecMemSpan {
    /// The element accesses in address order, each with the micro-op it
    /// becomes when the span is taken apart.
    pub elements: Vec<(MicroOpIdx, VecMemAddrOp)>,
    /// The bytes a load's access read, once the memory system served it
    /// or a store forwarded them.
    pub data: Option<Box<[u8]>>,
}

impl VecMemSpan {
    /// The address of the span's first byte.
    #[must_use]
    pub fn vaddr(&self) -> VirtAddr {
        self.elements.first().map_or(VirtAddr::new(0), |(_, first)| first.vaddr)
    }

    /// Bytes from the first element's first byte to the last element's last.
    #[must_use]
    pub fn bytes(&self) -> usize {
        let (Some((_, first)), Some((_, last))) = (self.elements.first(), self.elements.last())
        else {
            return 0;
        };
        (last.vaddr.val() - first.vaddr.val()) as usize + last.eew.bytes()
    }

    /// Where an element access's bytes start within the span.
    #[must_use]
    pub fn offset_of(&self, element: &VecMemAddrOp) -> usize {
        (element.vaddr.val() - self.vaddr().val()) as usize
    }
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
    /// The instruction's place in fetch order.
    pub seq: InstSeq,
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
    /// The instruction's place in fetch order.
    pub seq: InstSeq,
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
    /// Physical address of the upper half-word when a 32-bit instruction
    /// crosses a page; `None` when it lies within `paddr`'s page.
    pub upper_paddr: Option<PhysAddr>,
    /// Whether the branch predictor predicted taken.
    pub pred_taken: bool,
    /// Predicted target address.
    pub pred_target: u64,
    /// Trap during fetch (alignment, TLB fault).
    pub trap: Option<Trap>,
    /// Pipeline stage where the exception was detected.
    pub exception_stage: Option<ExceptionStage>,
    /// The instruction's place in fetch order.
    pub seq: InstSeq,
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
    /// The instruction's place in fetch order.
    pub seq: InstSeq,
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
    /// `vtype` CSR captured at dispatch time (O3: prevents stale reads when vsetvl is in-flight).
    pub vec_vtype: u64,
    /// `vl` CSR captured at dispatch time.
    pub vec_vl: u64,
    /// `vstart` CSR captured at dispatch time.
    pub vec_vstart: u64,
    /// `vxrm` CSR captured at dispatch time.
    pub vec_vxrm: u64,
    /// `frm` (FP rounding mode) CSR captured at dispatch time.
    pub vec_frm: u64,
}

/// Entry from Execute -> Memory1 latch.
#[derive(Clone, Debug, Default)]
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
    pub vec_mem: Option<VecMemAccess>,
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
    /// Raw load value when memory1 emitted a `MemReq` for this entry and the
    /// mailbox-drain has filled it in from the response. Holds the
    /// pre-sign-extension bytes; memory2 turns it into the final register
    /// value. For SB-forwarded loads, memory1 writes the forwarded value
    /// directly here. Zero for non-load ops.
    pub load_data: u64,
    /// Set by memory1 when a store-buffer hit replaced the cache request;
    /// memory2 sign-extends `load_data` without re-checking the SB.
    pub sb_forwarded: bool,
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
    /// The D-bit updates the access applies when it retires.
    pub dirty_updates: DirtyUpdates,
    /// Deferred SFENCE.VMA operands for commit-time TLB invalidation.
    pub sfence_vma: Option<SfenceVmaInfo>,
    /// Vector memory element metadata (flows through from `ExMem1Entry`).
    pub vec_mem: Option<VecMemAccess>,
    /// Write-log position when `load_data` was read from RAM; `None` for
    /// values that came from the store buffer or an MMIO device, and in
    /// single-hart systems.
    pub observed: Option<WriteSeq>,
}

impl ExMem1Entry {
    /// The result of executing `id`: `alu` and `store_data` for memory1,
    /// with no trap, FP flags or deferred side effects.
    pub const fn from_issue(id: &RenameIssueEntry, alu: u64, store_data: u64) -> Self {
        Self {
            rob_tag: id.rob_tag,
            pc: id.pc,
            inst: id.inst,
            inst_size: id.inst_size,
            rd: id.rd,
            rd_phys: id.rd_phys,
            alu,
            store_data,
            ctrl: id.ctrl,
            trap: None,
            exception_stage: None,
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        }
    }
}

impl Mem1Mem2Entry {
    /// Carries `ex`, its trap included, into memory2 at `vaddr`/`paddr`
    /// with nothing loaded yet.
    pub fn from_execute(ex: ExMem1Entry, vaddr: VirtAddr, paddr: PhysAddr) -> Self {
        Self {
            rob_tag: ex.rob_tag,
            pc: ex.pc,
            inst: ex.inst,
            inst_size: ex.inst_size,
            rd: ex.rd,
            rd_phys: ex.rd_phys,
            alu: ex.alu,
            vaddr,
            paddr,
            store_data: ex.store_data,
            load_data: 0,
            sb_forwarded: false,
            ctrl: ex.ctrl,
            trap: ex.trap,
            exception_stage: ex.exception_stage,
            fp_flags: ex.fp_flags,
            complete_cycle: 0,
            dirty_updates: DirtyUpdates::NONE,
            sfence_vma: ex.sfence_vma,
            vec_mem: ex.vec_mem,
            observed: None,
        }
    }
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
    /// The D-bit updates the access applies when it retires.
    pub dirty_updates: DirtyUpdates,
    /// Deferred SFENCE.VMA operands for commit-time TLB invalidation.
    pub sfence_vma: Option<SfenceVmaInfo>,
    /// Deferred LR/SC reservation action for commit-time application.
    pub lr_sc: Option<LrScRecord>,
    /// Vector memory element metadata (flows through from `ExMem1Entry`).
    pub vec_mem: Option<VecMemAccess>,
    /// Write-log position when the load value was read (see `Mem1Mem2Entry`).
    pub observed: Option<WriteSeq>,
}

#[cfg(test)]
mod latch_tests {
    use super::Latch;

    #[test]
    fn a_bundle_is_visible_after_the_latch_delay() {
        let mut latch = Latch::new(1);
        latch.push(10, [1, 2]);

        assert!(latch.ready(10).is_none(), "written this cycle, not visible yet");
        assert_eq!(latch.ready(11).map(|bundle| bundle.clone()), Some(vec![1, 2]));
    }

    #[test]
    fn a_zero_delay_latch_is_visible_the_cycle_it_is_written() {
        let mut latch = Latch::new(0);
        latch.push(5, [7]);

        assert_eq!(latch.take(5), vec![7]);
        assert!(latch.is_empty());
    }

    #[test]
    fn the_consumer_may_leave_part_of_the_bundle_for_later() {
        let mut latch = Latch::new(1);
        latch.push(0, [1, 2, 3]);

        let _ = latch.ready(1).map(|bundle| bundle.drain(..2));

        assert_eq!(latch.take(1), vec![3]);
    }

    #[test]
    fn take_before_the_delay_returns_nothing_and_keeps_the_bundle() {
        let mut latch = Latch::new(2);
        latch.push(0, [9]);

        assert!(latch.take(1).is_empty());
        assert_eq!(latch.len(), 1);
        assert_eq!(latch.take(2), vec![9]);
    }
}
