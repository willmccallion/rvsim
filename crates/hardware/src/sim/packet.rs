//! Typed packets routed through the event queue.
//!
//! Components communicate by scheduling packets on the global event queue
//! (see [`crate::sim::events`]). Each packet variant captures one class of
//! traffic — demand memory access, cache management, coherence, DRAM commands,
//! ordering fences. The receiving component's `Handle` impl matches on the
//! variant and reacts.
//!
//! Hot-path responses inline up to 8 bytes; cache-line payloads box their data
//! to keep the enum small.

use crate::common::{CoreId, LineAddr, PhysAddr, VirtAddr};
use crate::sim::components::ReqId;

/// Width of a single memory access in bytes.
///
/// `Line` is whatever the cache-line size is for this configuration (typically
/// 64 bytes). Sub-line widths are explicit so the receiver doesn't have to
/// pattern-match a raw byte count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessSize {
    /// 1-byte access.
    B1,
    /// 2-byte access.
    B2,
    /// 4-byte access.
    B4,
    /// 8-byte access.
    B8,
    /// One cache line.
    Line,
}

/// Payload carried in a `Write` operation. Inline storage for sub-line writes;
/// boxed slice for line-sized writes.
#[derive(Clone, Debug)]
pub enum WriteData {
    /// Up to 8 bytes packed into a `u64` (low-order bytes used per `AccessSize`).
    Small(u64),
    /// A full cache line (typically 64 bytes).
    Line(Box<[u8]>),
}

/// Response data payload for a load.
#[derive(Clone, Debug)]
pub enum MemRespData {
    /// Small inline payload (up to 8 bytes).
    Small(u64),
    /// A full cache line.
    Line(Box<[u8]>),
}

/// Atomic memory operation kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomicOp {
    /// `amoadd`.
    Add,
    /// `amoswap`.
    Swap,
    /// `amoxor`.
    Xor,
    /// `amoand`.
    And,
    /// `amoor`.
    Or,
    /// `amomin` (signed).
    Min,
    /// `amomax` (signed).
    Max,
    /// `amominu` (unsigned).
    MinU,
    /// `amomaxu` (unsigned).
    MaxU,
    /// `lr` (load-reserved).
    Lr,
    /// `sc` (store-conditional).
    Sc,
}

/// Memory operation kind on a `MemReq`.
#[derive(Clone, Debug)]
pub enum MemOp {
    /// Demand or speculative load.
    Read,
    /// Store with payload.
    Write {
        /// Bytes to write.
        data: WriteData,
    },
    /// Atomic read-modify-write.
    Atomic {
        /// The AMO sub-operation.
        op: AtomicOp,
        /// Source-register value for the AMO.
        data: u64,
    },
    /// Instruction fetch.
    Fetch,
    /// A whole line leaving a cache for the next level: its dirty data
    /// (`dirty`), or a clean victim handed to an exclusive lower level. The
    /// bytes are already in RAM; the packet carries the timing.
    Writeback {
        /// Whether the line was modified.
        dirty: bool,
    },
}

/// Cache level at which a request was satisfied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitLevel {
    /// L1 (instruction or data).
    L1,
    /// Private L2.
    L2,
    /// Shared LLC (L3, etc.).
    L3,
    /// Main memory.
    Dram,
    /// MMIO device.
    Mmio,
}

/// Logical cache level identifier for routing/stats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheLevel {
    /// L1 instruction cache.
    L1I,
    /// L1 data cache.
    L1D,
    /// Private L2 cache.
    L2,
    /// Shared LLC.
    L3,
}

/// Coherence snoop kind (used in Phase 8+ multi-core).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnoopKind {
    /// Reader is requesting shared access; current holder may keep `S`.
    Read,
    /// Reader is requesting exclusive access; current holder must invalidate.
    ReadForOwnership,
    /// Writeback request from an evicting cache.
    Writeback,
    /// Probe for line state without changing it.
    Probe,
}

/// MESI / MOESI coherence state.
///
/// `Owned` is set only by MOESI implementations; MESI never produces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MesiState {
    /// Line is dirty and held exclusively here.
    Modified,
    /// Line is dirty and held here; other caches may have clean copies.
    Owned,
    /// Line is clean and held exclusively here.
    Exclusive,
    /// Line is clean; may be held in other caches too.
    Shared,
    /// Line is not held.
    #[default]
    Invalid,
}

/// Fence kind, mirroring the RISC-V `fence` instruction `pred`/`succ` bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FenceKind {
    /// `fence.i` (instruction fence).
    I,
    /// `fence r,...`.
    R,
    /// `fence w,...`.
    W,
    /// `fence rw,rw`.
    RW,
    /// TSO ordering hint.
    Tso,
}

/// Scope of a fence — which agents and which memory regions it orders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FenceScope {
    /// Local hart only.
    Local,
    /// Visible across all harts on this core (SMT siblings).
    Core,
    /// Visible across the system (all cores, all devices).
    System,
}

/// DRAM command kind (used by `Packet::DramCmd` in Phase 7+).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DramCmdKind {
    /// ACTIVATE: open a row into the row buffer.
    Activate,
    /// PRECHARGE: close the open row.
    Precharge,
    /// PRECHARGE-ALL (or same-bank): close every open row a refresh covers.
    PrechargeAll,
    /// READ from the open row.
    Read,
    /// WRITE to the open row.
    Write,
    /// REFRESH a rank or per-bank group.
    Refresh,
    /// Power-down entry (precharge or active power-down).
    PowerDownEntry,
    /// Power-down exit; commands resume after tXP.
    PowerDownExit,
}

/// A typed packet routed through the event queue.
#[derive(Clone, Debug)]
pub enum Packet {
    /// Demand or speculative memory request issued by the pipeline (or a
    /// cache forwarding a miss downstream).
    MemReq {
        /// Originator's correlator.
        req_id: ReqId,
        /// Post-translation routing key.
        paddr: PhysAddr,
        /// Pre-translation address, retained on the fetch path for fault
        /// reporting (the trap handler reads `stval` from the original VA).
        vaddr: Option<VirtAddr>,
        /// Width of the access.
        size: AccessSize,
        /// Read / write / atomic / fetch.
        op: MemOp,
    },
    /// Response to a `MemReq`. Carries data + the level that serviced the hit
    /// for stat correlation.
    MemResp {
        /// Originator's correlator (matches the request).
        req_id: ReqId,
        /// Cache-line identifier of the response.
        line_addr: LineAddr,
        /// Loaded bytes.
        data: MemRespData,
        /// Cache level at which the hit occurred.
        hit_level: HitLevel,
    },
    /// Invalidate a cache line (back-invalidation, FENCE.VMA, coherence-driven).
    CacheInval {
        /// Line to invalidate.
        line_addr: LineAddr,
    },
    /// Clean a cache line (write back dirty data, retain valid in clean state).
    CacheClean {
        /// Line to clean.
        line_addr: LineAddr,
    },
    /// Prefetcher-generated request.
    Prefetch {
        /// Line to fetch.
        line_addr: LineAddr,
        /// Cache level that issued the prefetch.
        source_level: CacheLevel,
    },
    /// Coherence snoop request from a remote core.
    SnoopReq {
        /// Correlator.
        req_id: ReqId,
        /// Line being snooped.
        line_addr: LineAddr,
        /// What the requester is asking for.
        kind: SnoopKind,
        /// Originating core.
        requester: CoreId,
    },
    /// Coherence snoop response back to the requester.
    SnoopResp {
        /// Matches the request.
        req_id: ReqId,
        /// Line in question.
        line_addr: LineAddr,
        /// Responder's resulting state for the line.
        state: MesiState,
        /// Optional dirty data if the responder is sourcing it.
        data: Option<MemRespData>,
    },
    /// DRAM-internal command (visible for command-level stats).
    DramCmd {
        /// Channel index.
        channel: u8,
        /// Rank index within the channel.
        rank: u8,
        /// Bank index within the rank.
        bank: u8,
        /// Command kind.
        kind: DramCmdKind,
        /// Row index for activate / precharge.
        row: u32,
    },
    /// Self-scheduled refresh tick on a rank.
    RefreshTick {
        /// Channel index.
        channel: u8,
        /// Rank index.
        rank: u8,
    },
    /// Ordering fence; targets the LSU / store buffer / write-combining buffer.
    Fence {
        /// Kind of fence (I, R, W, RW, TSO).
        kind: FenceKind,
        /// Scope of fence visibility.
        scope: FenceScope,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writedata_small_round_trip() {
        let w = WriteData::Small(0x12_34_56_78);
        match w {
            WriteData::Small(v) => assert_eq!(v, 0x12_34_56_78),
            WriteData::Line(_) => panic!("wrong variant"),
        }
    }

    #[test]
    fn memresp_line_payload() {
        let bytes: Box<[u8]> = vec![0xAA; 64].into_boxed_slice();
        let r = MemRespData::Line(bytes);
        if let MemRespData::Line(b) = r {
            assert_eq!(b.len(), 64);
            assert_eq!(b[0], 0xAA);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn mesistate_default_is_invalid() {
        assert_eq!(MesiState::default(), MesiState::Invalid);
    }
}
