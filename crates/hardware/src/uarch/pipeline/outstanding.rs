//! Outstanding-request tracking for the event-driven pipeline.
//!
//! Each in-flight memory operation issued by the pipeline (instruction fetch,
//! demand load, store write-allocate, atomic RMW, page-table walk) is recorded
//! here keyed by its [`ReqId`]. When the matching
//! [`Packet::MemResp`](crate::sim::packet::Packet::MemResp) lands in the
//! pipeline's mailbox, the drain stage looks up the entry, finishes the
//! work the original stage couldn't (apply sign extension, complete the
//! ROB, advance the walk, push a fetch latch entry, …) and forgets it.

use crate::arch::translation::TranslationResult;
use crate::common::{LineAddr, PhysAddr, VirtAddr};
use crate::sim::packet::MemRespData;
use crate::system::state::write_log::WriteSeq;
use crate::uarch::mmu::ptw::WalkState;
use crate::uarch::pipeline::latches::{ExMem1Entry, Fetch1Fetch2Entry, VecMemAccess, VecMemTarget};

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
    /// The D-bit updates the access applies when it retires (an AMO's).
    pub dirty_updates: crate::arch::translation::DirtyUpdates,
    /// The access reads a device, so it was issued non-speculatively from
    /// the ROB head and must complete before anything pre-empts it.
    pub side_effecting: bool,
    /// What the access's cache requests have read so far.
    pub parts: LoadParts,
}

impl OutstandingLoad {
    /// Gives a vector span load the bytes its access read.
    pub fn set_span_data(&mut self, bytes: Box<[u8]>) {
        if let Some(VecMemAccess { target: VecMemTarget::Span(span), .. }) =
            self.entry.vec_mem.as_mut()
        {
            span.data = Some(bytes);
        }
    }
}

/// What one request of a load read, when the memory system served it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartRead {
    /// The bytes read, zero-extended.
    pub value: u64,
    /// The write order they reflect; `None` with a single hart or from a
    /// device.
    pub observed: Option<WriteSeq>,
}

impl PartRead {
    /// What a response to one of the load's requests carries.
    #[must_use]
    pub const fn of(data: &MemRespData) -> Self {
        match data {
            MemRespData::Performed { value, observed } => {
                Self { value: *value, observed: *observed }
            }
            MemRespData::Small(value) => Self { value: *value, observed: None },
            MemRespData::PerformedBytes { observed, .. } => Self { value: 0, observed: *observed },
            MemRespData::Line(_) => Self { value: 0, observed: None },
        }
    }
}

/// The reads of a load's requests: one, or two for an access that
/// straddles a cache line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadParts {
    /// The access lies in one line.
    Whole(Option<PartRead>),
    /// The access straddles a line boundary with `low_bytes` bytes below it.
    Split {
        /// Bytes of the access in the first line.
        low_bytes: u8,
        /// The first line's read.
        low: Option<PartRead>,
        /// The second line's read.
        high: Option<PartRead>,
    },
}

impl LoadParts {
    /// Records the read of the request for the second line when `high`,
    /// else of the (first) line.
    pub const fn record(&mut self, high: bool, read: PartRead) {
        match self {
            Self::Whole(whole) => *whole = Some(read),
            Self::Split { low, .. } if !high => *low = Some(read),
            Self::Split { high: slot, .. } => *slot = Some(read),
        }
    }

    /// The whole access's read once every request has answered. A split
    /// read reflects the earlier of its two parts' write orders.
    #[must_use]
    pub fn assembled(&self) -> Option<PartRead> {
        match *self {
            Self::Whole(read) => read,
            Self::Split { low_bytes, low: Some(low), high: Some(high) } => Some(PartRead {
                value: low.value | (high.value << (8 * u32::from(low_bytes))),
                observed: match (low.observed, high.observed) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                },
            }),
            Self::Split { .. } => None,
        }
    }
}

/// A memory access that already holds translations, from an L2 TLB hit or a
/// completed page-table walk: it proceeds at `ready_cycle` with them.
#[derive(Clone, Debug)]
pub struct DelayedAccess {
    /// Cycle at which the access may continue.
    pub ready_cycle: u64,
    /// The access.
    pub entry: ExMem1Entry,
    /// Its translations so far, latency already paid.
    pub translations: PageTranslations,
}

/// The translations a memory access has obtained so far: its first page's
/// and, for an access that crosses into the next page, that page's.
#[derive(Clone, Debug, Default)]
pub struct PageTranslations {
    /// The page holding the access's first byte.
    pub first: Option<TranslationResult>,
    /// The next page, for an access that crosses into it.
    pub second: Option<TranslationResult>,
}

/// Which buffer a write request's acknowledgement goes back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreOwner {
    /// A scalar store, which keeps its store-buffer slot until acknowledged.
    StoreBuffer,
    /// A vector store, which keeps its vector-store-buffer entry until
    /// every write of it is acknowledged.
    VecStoreBuffer,
    /// A line the write-combining buffer sent, which barriers wait for.
    WriteCombining,
    /// A write nothing waits for: a line writeback, a PTE update, a CBO's
    /// writes, or a store written at once for a checkpoint.
    Untracked,
}

/// A write request awaiting its acknowledgement.
#[derive(Clone, Debug)]
pub struct OutstandingStore {
    /// Who is told when it is acknowledged.
    pub owner: StoreOwner,
    /// Physical address written.
    pub paddr: PhysAddr,
}

/// A page-table walk in flight.
///
/// `state` holds the live walker state (current level, page-table root PPN,
/// access info). `pte_addr` is the physical address of the PTE the walker
/// is currently waiting on — the drain stage reads its 64-bit value from
/// the RAM fast path before handing it to
/// [`CoreCtx::translate_continue`](crate::system::CoreCtx::translate_continue).
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
    /// An instruction fetch waiting on the translation of its first or
    /// second half-word. When the walk completes, fetch1 fetches the
    /// instruction again, or a fault drains as a one-entry fetch group
    /// under `fetch_seq`.
    Fetch {
        /// Sequence number reserved for the group at park time so it drains
        /// after the instructions fetch1 issued before it.
        fetch_seq: u64,
        /// The instruction whose translation is outstanding.
        entry: Fetch1Fetch2Entry,
    },
    /// A load or store waiting on the translation of one of its pages.
    /// When the walk completes, memory1 continues the access with the
    /// walk's result and whatever it had translated before.
    LoadStore {
        /// The access.
        entry: ExMem1Entry,
        /// The translations it obtained before this walk.
        translations: Box<PageTranslations>,
    },
}

/// A load answered from the store buffer: its data is known at once, but
/// it reaches memory2 only after the L1D hit latency, like any other load.
#[derive(Clone, Debug)]
pub struct ForwardedLoad {
    /// Cycle the data would have come back from the cache.
    pub ready_cycle: u64,
    /// The completed memory1 entry.
    pub entry: crate::uarch::pipeline::latches::Mem1Mem2Entry,
}
