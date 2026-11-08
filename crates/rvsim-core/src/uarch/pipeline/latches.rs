//! Pipeline latch structures for inter-stage communication.
//!
//! This module defines the entry types carried between the 10-stage pipeline:
//! Fetch1 → Fetch2 → Decode → Rename → Issue → Execute → Mem1 → Mem2 → Writeback → Commit.
//!
//! 1. **Instruction Flow:** Structures for carrying state between pipeline stages.
//! 2. **Superscalar Support:** Multi-entry latches for wide-issue configurations.
//! 3. **Trap Propagation:** Carrying architectural exceptions and interrupts through the pipeline.

use std::collections::VecDeque;

use crate::arch::reservation::LrScRecord;
use crate::arch::translation::{DirtyUpdates, SfenceVmaInfo};
use crate::common::{InstSeq, PhysAddr, VirtAddr};
use crate::exec::compute::vector::mem::ElementAccess;
use crate::exec::inst::Inst;
use crate::exec::signals::{ControlFlow, ControlSignals};
use crate::isa::fp::FpFlags;
use crate::isa::instruction::InstSize;
use crate::isa::op::AtomicOp;
use crate::isa::privileged::Trap;
use crate::isa::reg::RegIdx;
use crate::isa::rvv::{ElemIdx, Sew, VectorConfig, VectorRounding};
use crate::sim::memory::write_log::WriteSeq;
#[cfg(feature = "commit-log")]
use crate::uarch::pipeline::commit_log::MemEffect;
#[cfg(feature = "commit-log")]
use crate::uarch::pipeline::commit_log::VectorElementAccess;
use crate::uarch::pipeline::rename::prf::PhysReg;
use crate::uarch::pipeline::rob::RobTag;

/// A pipeline register between two stages.
///
/// The producer writes a bundle of entries, which becomes visible to the
/// consumer `delay` cycles after it was written, and the latch holds one
/// bundle per cycle of delay in flight, as gem5's `TimeBuffer` does, so a
/// longer delay costs cycles, not throughput. The consumer empties the
/// oldest visible bundle, and the producer writes only while the latch has
/// room, which is how a stall propagates backwards. Bundles written in one
/// cycle form one bundle.
#[derive(Clone, Debug)]
pub struct Latch<T> {
    bundles: VecDeque<Bundle<T>>,
    delay: u64,
}

#[derive(Clone, Debug)]
struct Bundle<T> {
    ready_at: u64,
    entries: Vec<T>,
}

impl<T> Latch<T> {
    /// An empty latch whose bundles become visible `delay` cycles after
    /// they are written.
    #[must_use]
    pub const fn new(delay: u64) -> Self {
        Self { bundles: VecDeque::new(), delay }
    }

    /// Cycles a bundle takes to become visible.
    #[must_use]
    pub const fn delay(&self) -> u64 {
        self.delay
    }

    /// True when no entry is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bundles.iter().all(|bundle| bundle.entries.is_empty())
    }

    /// True when the producer may write this cycle: fewer bundles are in
    /// flight than the delay (one, for a delay of 0 or 1).
    #[must_use]
    pub fn has_room(&self) -> bool {
        let in_flight = self.bundles.iter().filter(|bundle| !bundle.entries.is_empty()).count();
        in_flight < usize::try_from(self.delay.max(1)).unwrap_or(usize::MAX)
    }

    #[cfg(test)]
    /// Entries held, visible or not.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bundles.iter().map(|bundle| bundle.entries.len()).sum()
    }

    /// Adds `entries`, visible from `now + delay`, to the bundle written
    /// this cycle or as a new one.
    pub fn push(&mut self, now: u64, entries: impl IntoIterator<Item = T>) {
        let ready_at = now.saturating_add(self.delay);
        match self.bundles.back_mut() {
            Some(bundle) if bundle.ready_at == ready_at => bundle.entries.extend(entries),
            _ => {
                let entries: Vec<T> = entries.into_iter().collect();
                if !entries.is_empty() {
                    self.bundles.push_back(Bundle { ready_at, entries });
                }
            }
        }
    }

    /// The oldest bundle, if the consumer may see it at `now`.
    #[must_use]
    pub fn ready(&mut self, now: u64) -> Option<&mut Vec<T>> {
        self.drop_consumed();
        self.bundles
            .front_mut()
            .filter(|bundle| bundle.ready_at <= now)
            .map(|bundle| &mut bundle.entries)
    }

    /// Takes the oldest bundle if the consumer may see it at `now`.
    pub fn take(&mut self, now: u64) -> Vec<T> {
        let taken = self.ready(now).map(std::mem::take).unwrap_or_default();
        self.drop_consumed();
        taken
    }

    /// Drops every bundle.
    pub fn clear(&mut self) {
        self.bundles.clear();
    }

    /// The entries held, oldest first, for a snapshot.
    pub fn entries(&self) -> impl Iterator<Item = &T> {
        self.bundles.iter().flat_map(|bundle| bundle.entries.iter())
    }

    /// Forgets the bundles the consumer has emptied.
    fn drop_consumed(&mut self) {
        while self.bundles.front().is_some_and(|bundle| bundle.entries.is_empty()) {
            let _ = self.bundles.pop_front();
        }
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

/// What a vector memory micro-op accesses.
#[derive(Clone, Debug)]
pub enum VecMemTarget {
    /// One element, or one field of a segment element.
    Element {
        /// Element index within the vector register group.
        elem_idx: ElemIdx,
        /// Effective element width for this access.
        eew: Sew,
        /// Which register of the destination group the element's data
        /// lands in, counting from `vd`.
        dest_slot: usize,
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
    pub elements: Vec<(MicroOpIdx, ElementAccess)>,
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
    pub fn offset_of(&self, element: &ElementAccess) -> usize {
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
    pub pc: VirtAddr,
    /// Its encoding, expanded to 32 bits when it is compressed.
    pub bits: u32,
    /// Size of the instruction in bytes (2 for compressed, 4 for standard).
    pub inst_size: InstSize,
    /// Where the branch predictor sent fetch next, when it predicted
    /// this instruction taken.
    pub predicted_target: Option<VirtAddr>,
    /// Trap that occurred during fetch, if any.
    pub trap: Option<Trap>,
    /// The instruction's place in fetch order.
    pub seq: InstSeq,
}

/// Entry in the ID/EX pipeline latch (Decode to Execute stage): the decoded
/// instruction with its operand values, fetch's prediction for it, and any
/// trap found so far.
#[derive(Clone, Default, Debug)]
pub struct IdExEntry {
    /// The decoded instruction and its operand values.
    pub inst: Inst,
    /// Trap that occurred during decode, if any.
    pub trap: Option<Trap>,
    /// Where the branch predictor sent fetch next, when it predicted
    /// this instruction taken.
    pub predicted_target: Option<VirtAddr>,
    /// The instruction's place in fetch order.
    pub seq: InstSeq,
}

/// Entry in Fetch1 -> Fetch2 latch.
///
/// Carries PC and I-TLB/branch prediction results from PC generation
/// into the I-cache access stage.
#[derive(Clone, Default, Debug)]
pub struct Fetch1Fetch2Entry {
    /// Program counter.
    pub pc: VirtAddr,
    /// Physical address after I-TLB lookup.
    pub paddr: PhysAddr,
    /// Physical address of the upper half-word when a 32-bit instruction
    /// crosses a page; `None` when it lies within `paddr`'s page.
    pub upper_paddr: Option<PhysAddr>,
    /// Where the branch predictor sent fetch next, when it predicted
    /// this instruction taken.
    pub predicted_target: Option<VirtAddr>,
    /// Trap during fetch (alignment, TLB fault).
    pub trap: Option<Trap>,
    /// The instruction's place in fetch order.
    pub seq: InstSeq,
}

/// Entry from Rename -> Issue (also used as Issue -> Execute input).
///
/// This is the fully-decoded, ROB-tagged instruction entering the backend
/// pipeline, with the registers `R` its backend tracks its operands by.
#[derive(Clone, Default, Debug)]
pub struct RenameIssueEntry<R> {
    /// ROB tag assigned during rename.
    pub rob_tag: RobTag,
    /// The decoded instruction and its operand values.
    pub inst: Inst,
    /// Trap from earlier stages.
    pub trap: Option<Trap>,
    /// Where the branch predictor sent fetch next, when it predicted
    /// this instruction taken.
    pub predicted_target: Option<VirtAddr>,
    /// The instruction's place in fetch order.
    pub seq: InstSeq,
    /// The vector configuration at rename, so a `vsetvl` still in flight
    /// cannot change what the instruction executes with.
    pub vector_config: VectorConfig,
    /// The vector rounding modes at rename.
    pub vector_rounding: VectorRounding,
    /// Where its operands come from and its result goes.
    pub regs: R,
}

impl<R: IssueRegisters> RenameIssueEntry<R> {
    /// This entry as a snapshot shows it.
    pub fn snapshot(&self) -> RenameIssueEntry<SourceReadiness> {
        RenameIssueEntry {
            rob_tag: self.rob_tag,
            inst: self.inst,
            trap: self.trap.clone(),
            predicted_target: self.predicted_target,
            seq: self.seq,
            vector_config: self.vector_config,
            vector_rounding: self.vector_rounding,
            regs: self.regs.readiness(),
        }
    }
}

/// The registers a backend tracks an issuing instruction's operands by.
pub trait IssueRegisters: Clone + std::fmt::Debug {
    /// The physical register its result goes to: x0's, which discards it,
    /// for an instruction with no destination or a backend that renames
    /// nothing.
    fn rd_phys(&self) -> PhysReg;

    /// Whether rs1 and rs2 were known at rename, as a snapshot shows them.
    fn readiness(&self) -> SourceReadiness;
}

/// Whether an instruction's rs1 and rs2 were known when it was renamed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceReadiness {
    /// rs1's value was known.
    pub rs1: bool,
    /// rs2's value was known.
    pub rs2: bool,
}

/// What every latch from execute to writeback carries of an instruction
/// unchanged: which it is, what execute computed, its fault, and the
/// effects it defers to commit.
#[derive(Clone, Debug, Default)]
pub struct InFlightOp {
    /// ROB tag.
    pub rob_tag: RobTag,
    /// Program counter.
    pub pc: VirtAddr,
    /// Its encoding, expanded to 32 bits when it is compressed.
    pub bits: u32,
    /// Instruction size.
    pub inst_size: InstSize,
    /// Destination register.
    pub rd: RegIdx,
    /// Physical destination register (O3 PRF path).
    pub rd_phys: PhysReg,
    /// ALU result / memory virtual address.
    pub alu: u64,
    /// Control signals.
    pub ctrl: ControlSignals,
    /// Trap from execute or a memory stage.
    pub trap: Option<Trap>,
    /// FP exception flags from this instruction (deferred to commit).
    pub fp_flags: FpFlags,
    /// Deferred SFENCE.VMA operands for commit-time TLB invalidation.
    pub sfence_vma: Option<SfenceVmaInfo>,
    /// Vector memory element metadata (None for scalar ops).
    pub vec_mem: Option<VecMemAccess>,
}

/// Entry from Execute -> Memory1 latch.
#[derive(Clone, Debug, Default)]
pub struct ExMem1Entry {
    /// The instruction and what execute made of it.
    pub op: InFlightOp,
    /// Store data (rs2 value).
    pub store_data: u64,
    /// A store's address half: `store_data` is not its data, which its
    /// data half delivers to the store buffer separately.
    pub store_data_follows: bool,
    /// Memory1 sent it back to wait last cycle, so waiting on is the same
    /// wait.
    pub replaying: bool,
}

/// Entry from Memory1 -> Memory2 latch.
#[derive(Clone, Default, Debug)]
pub struct Mem1Mem2Entry {
    /// The instruction and what execute made of it.
    pub op: InFlightOp,
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
    /// The D-bit updates the access applies when it retires.
    pub dirty_updates: DirtyUpdates,
    /// Write-log position when `load_data` was read from RAM; `None` for
    /// values that came from the store buffer or an MMIO device, and in
    /// single-hart systems.
    pub observed: Option<WriteSeq>,
}

impl ExMem1Entry {
    /// The virtual address a memory access computed into `alu`.
    pub const fn vaddr(&self) -> VirtAddr {
        VirtAddr::new(self.op.alu)
    }

    /// The address of the instruction after this one.
    pub const fn next_pc(&self) -> VirtAddr {
        self.op.pc.wrapping_add(self.op.inst_size.as_u64())
    }

    /// The value a non-memory instruction writes to `rd`: a jump's return
    /// address, else the ALU result.
    pub const fn register_value(&self) -> u64 {
        if matches!(self.op.ctrl.control_flow, ControlFlow::Jump(_)) {
            self.next_pc().val()
        } else {
            self.op.alu
        }
    }

    /// The result of executing `id`: `alu` and `store_data` for memory1,
    /// with no trap, FP flags or deferred side effects.
    pub fn from_issue<R: IssueRegisters>(
        issued: &RenameIssueEntry<R>,
        alu: u64,
        store_data: u64,
    ) -> Self {
        Self {
            op: InFlightOp {
                rob_tag: issued.rob_tag,
                pc: issued.inst.pc,
                bits: issued.inst.bits,
                inst_size: issued.inst.size,
                rd: issued.inst.rd,
                rd_phys: issued.regs.rd_phys(),
                alu,
                ctrl: issued.inst.ctrl,
                trap: None,
                fp_flags: FpFlags::NONE,
                sfence_vma: None,
                vec_mem: None,
            },
            store_data,
            store_data_follows: false,
            replaying: false,
        }
    }
}

impl Mem1Mem2Entry {
    /// Carries `ex`, its trap included, into memory2 at `vaddr`/`paddr`
    /// with nothing loaded yet.
    pub fn from_execute(ex: ExMem1Entry, vaddr: VirtAddr, paddr: PhysAddr) -> Self {
        Self {
            op: ex.op,
            vaddr,
            paddr,
            store_data: ex.store_data,
            load_data: 0,
            sb_forwarded: false,
            dirty_updates: DirtyUpdates::NONE,
            observed: None,
        }
    }
}

/// Entry from Memory2 -> Writeback latch.
#[derive(Clone, Default, Debug)]
pub struct Mem2WbEntry {
    /// The instruction and what execute made of it.
    pub op: InFlightOp,
    /// Loaded data (for load instructions).
    pub load_data: u64,
    /// The D-bit updates the access applies when it retires.
    pub dirty_updates: DirtyUpdates,
    /// Deferred LR/SC reservation action for commit-time application.
    pub lr_sc: Option<LrScRecord>,
    /// Write-log position when the load value was read (see `Mem1Mem2Entry`).
    pub observed: Option<WriteSeq>,
    /// The address a load, store or atomic touched, for stops on a watched
    /// range.
    pub mem_access: Option<MemAccess>,
    /// The access a load, LR, SC or AMO performed, for the commit log.
    #[cfg(feature = "commit-log")]
    pub mem_effect: Option<MemEffect>,
    /// The element accesses a vector memory micro-op made, for the commit log.
    #[cfg(feature = "commit-log")]
    pub vec_mem_effects: Vec<VectorElementAccess>,
}

impl Mem2WbEntry {
    /// The address of the instruction after this one.
    pub const fn next_pc(&self) -> VirtAddr {
        self.op.pc.wrapping_add(self.op.inst_size.as_u64())
    }

    /// The value the instruction writes to `rd`: what a load or atomic
    /// read, a jump's return address, or else the ALU result.
    pub const fn written_value(&self) -> u64 {
        if self.op.ctrl.mem_read || self.op.ctrl.atomic_op.is_some() {
            self.load_data
        } else if matches!(self.op.ctrl.control_flow, ControlFlow::Jump(_)) {
            self.next_pc().val()
        } else {
            self.op.alu
        }
    }

    /// Carries `mem`, its trap included, into writeback with its final
    /// register value and the reservation record it leaves for commit.
    pub fn from_memory2(mem: Mem1Mem2Entry, load_data: u64, lr_sc: Option<LrScRecord>) -> Self {
        let mem_access = MemAccess::of_memory2(&mem);
        #[cfg(feature = "commit-log")]
        let mem_effect = MemEffect::of_memory2(&mem);
        #[cfg(feature = "commit-log")]
        let vec_mem_effects = VectorElementAccess::of_memory2(&mem);
        Self {
            op: mem.op,
            mem_access,
            load_data,
            dirty_updates: mem.dirty_updates,
            lr_sc,
            observed: mem.observed,
            #[cfg(feature = "commit-log")]
            mem_effect,
            #[cfg(feature = "commit-log")]
            vec_mem_effects,
        }
    }
}

/// The address a load, store or atomic touched, for stops on a watched
/// range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemAccess {
    /// Where the access began.
    pub vaddr: VirtAddr,
    /// How many bytes it covered.
    pub bytes: u64,
    /// Whether it read memory.
    pub read: bool,
    /// Whether it wrote memory.
    pub write: bool,
}

impl MemAccess {
    /// The scalar access leaving memory2, if it performed one.
    #[must_use]
    pub const fn of_memory2(mem: &Mem1Mem2Entry) -> Option<Self> {
        if mem.op.trap.is_some() || mem.op.vec_mem.is_some() || mem.op.ctrl.system_op.is_cbo() {
            return None;
        }
        let (read, write) = match mem.op.ctrl.atomic_op {
            Some(AtomicOp::Lr) => (true, false),
            Some(AtomicOp::Sc) => (false, mem.load_data == 0),
            Some(_) => (true, true),
            None => (mem.op.ctrl.mem_read, mem.op.ctrl.mem_write),
        };
        if !read && !write {
            return None;
        }
        Some(Self { vaddr: mem.vaddr, bytes: mem.op.ctrl.width.bytes(), read, write })
    }
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
    fn a_two_cycle_latch_carries_a_bundle_per_cycle() {
        let mut latch = Latch::new(2);
        latch.push(0, [1]);
        assert!(latch.has_room(), "a second bundle is in flight behind the first");
        latch.push(1, [2]);
        assert!(!latch.has_room(), "two cycles of delay hold two bundles");

        assert_eq!(latch.take(2), vec![1]);
        assert!(latch.has_room());
        assert_eq!(latch.take(3), vec![2]);
        assert!(latch.is_empty());
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
