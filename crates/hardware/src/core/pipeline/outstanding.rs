//! Outstanding-request tracking for the event-driven pipeline.
//!
//! Each in-flight memory operation issued by the pipeline (instruction fetch,
//! demand load, store write-allocate, atomic RMW, page-table walk) is recorded
//! here keyed by its [`ReqId`]. When the matching
//! [`Packet::MemResp`](crate::sim::packet::Packet::MemResp) lands in the
//! pipeline's mailbox, the drain stage looks up the entry, finishes the
//! work the original stage couldn't (apply sign extension, complete the
//! ROB, advance the walk, push a fetch latch entry, …) and forgets it.

use crate::common::{LineAddr, PhysAddr, VirtAddr};
use crate::core::pipeline::frontend::fetch1::FetchWalkHalf;
use crate::core::pipeline::latches::{ExMem1Entry, Fetch1Fetch2Entry};
use crate::core::pipeline::rob::RobTag;
use crate::core::units::mmu::ptw::WalkState;

/// One instruction-fetch group: the instructions fetch1 produced in a single
/// cycle, all from one cache line.
///
/// Fetch1 reads instruction bytes synchronously from the RAM fast path, so
/// every [`Fetch1Fetch2Entry`] is fully formed at issue time. What the group
/// waits for is the I-cache: one line-sized
/// [`MemReq`](crate::sim::packet::Packet::MemReq) per group, or nothing at
/// all when the fetch buffer already holds `line`.
#[derive(Clone, Debug)]
pub struct OutstandingFetch {
    /// Program-order sequence number. The mailbox-drain stage releases groups
    /// to the fetch1→fetch2 latch in `fetch_seq` order, so a group that
    /// completes early (fetch-buffer hit, or a walk that finished while an
    /// older line was still missing) waits for its predecessors.
    pub fetch_seq: u64,
    /// Cache line the group's instructions live in. `None` when the group
    /// holds only a fetch-fault entry, which needs no I-cache access.
    pub line: Option<LineAddr>,
    /// Instructions in program order.
    pub entries: Vec<Fetch1Fetch2Entry>,
}

/// A demand load (or atomic / LR) awaiting its `MemResp`.
///
/// The full [`ExMem1Entry`] is kept so the drain stage can replay the load
/// completion logic without recomputing translation, alignment, or
/// control-signal lookups. `paddr` is the translated physical address;
/// `vaddr` is retained for trace + load-queue updates.
#[derive(Clone, Debug)]
pub struct OutstandingLoad {
    /// Original Execute→Memory1 entry, carrying ctrl signals, rd / `rd_phys`,
    /// pc, inst, `fp_flags`, `vec_mem`, `sfence_vma`, and `store_data` (used by AMO
    /// as the second operand).
    pub entry: ExMem1Entry,
    /// Translated physical address.
    pub paddr: PhysAddr,
    /// Pre-translation virtual address (kept for the load queue's address
    /// field and for trace output).
    pub vaddr: VirtAddr,
    /// Deferred PTE A/D bit update from translation (applied at commit).
    pub pte_update: Option<crate::common::PteUpdate>,
    /// The access reads a device, so it was issued non-speculatively from
    /// the ROB head and must complete before anything pre-empts it.
    pub side_effecting: bool,
    /// Cache requests still to be answered: two for an access that
    /// straddles a line, otherwise one.
    pub parts_outstanding: u8,
}

/// A memory access whose translation hit the L2 TLB: it proceeds once the
/// L2 TLB's latency has elapsed, with the translation it already has.
#[derive(Clone, Debug)]
pub struct DelayedAccess {
    /// Cycle at which the access may continue.
    pub ready_cycle: u64,
    /// The access.
    pub entry: ExMem1Entry,
    /// Its translation, latency already paid.
    pub translation: crate::common::TranslationResult,
}

/// A store awaiting cache write-allocate acknowledgment.
///
/// The store itself resolves its store buffer slot inline at memory1 — this
/// entry tracks the cache-side write-allocate `MemReq` so the LSU's
/// outstanding-count is accurate for back-pressure decisions.
#[derive(Clone, Debug)]
pub struct OutstandingStore {
    /// ROB tag of the store.
    pub rob_tag: RobTag,
    /// Translated physical address of the store.
    pub paddr: PhysAddr,
}

/// A page-table walk in flight.
///
/// `state` holds the live walker state (current level, page-table root PPN,
/// access info). `pte_addr` is the physical address of the PTE the walker
/// is currently waiting on — the drain stage reads its 64-bit value from
/// the RAM fast path before handing it to
/// [`CoreCtx::translate_continue`](crate::sim::CoreCtx::translate_continue).
/// `continuation` says what to do once the walk completes.
#[derive(Clone, Debug)]
pub struct OutstandingWalk {
    /// PTW state being advanced.
    pub state: WalkState,
    /// Physical address of the PTE that the outstanding `MemReq` is reading.
    pub pte_addr: PhysAddr,
    /// What to do once the walk completes.
    pub continuation: WalkContinuation,
}

/// What an in-progress walk resumes once it completes.
#[derive(Clone, Debug)]
pub enum WalkContinuation {
    /// An instruction fetch waiting on its translation. When the walk
    /// completes the instruction is dispatched as a one-entry fetch group
    /// under `fetch_seq`.
    Fetch {
        /// Sequence number reserved for the group at park time so it drains
        /// after the instructions fetch1 issued before it.
        fetch_seq: u64,
        /// The instruction whose translation is outstanding.
        entry: Fetch1Fetch2Entry,
        /// Which half-word the walk translates. A `Lower` walk supplies the
        /// entry's `paddr`; an `Upper` walk only warms the TLB for fetch2's
        /// re-translation of the page-crossing upper half.
        half: FetchWalkHalf,
    },
    /// A demand load or store waiting on its translation. The
    /// `ExMem1Entry` is re-injected into the Execute→Memory1 latch so
    /// memory1 re-runs with the (now TLB-resident) translation.
    LoadStore(ExMem1Entry),
}

/// A load answered from the store buffer: its data is known at once, but
/// it reaches memory2 only after the L1D hit latency, like any other load.
#[derive(Clone, Debug)]
pub struct ForwardedLoad {
    /// Cycle the data would have come back from the cache.
    pub ready_cycle: u64,
    /// The completed memory1 entry.
    pub entry: crate::core::pipeline::latches::Mem1Mem2Entry,
}
