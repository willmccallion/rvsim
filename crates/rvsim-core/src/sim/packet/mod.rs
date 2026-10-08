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

pub mod coherence;

use crate::common::{HartId, LineAddr, PhysAddr, VirtAddr};
use crate::isa::op::AtomicOp;
use crate::sim::components::ReqId;
use crate::sim::memory::write_log::WriteSeq;
use crate::sim::packet::coherence::CoherenceMsg;

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
    /// The bytes (1 to 7) of an access that fall on one side of the cache
    /// line boundary it straddles.
    Part(u8),
    /// One cache line.
    Line,
    /// A vector access's contiguous bytes (1 to 64) within one cache line:
    /// a hart's access, never a line fill.
    Span(u8),
}

impl AccessSize {
    /// Bytes the access moves; a line is the 64-byte line the bus carries.
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::B1 => 1,
            Self::B2 => 2,
            Self::B4 => 4,
            Self::B8 => 8,
            Self::Part(bytes) | Self::Span(bytes) => bytes as usize,
            Self::Line => 64,
        }
    }

    /// The size of an access of `bytes` (at most 8) bytes.
    #[must_use]
    pub const fn of_bytes(bytes: usize) -> Self {
        match bytes {
            1 => Self::B1,
            2 => Self::B2,
            4 => Self::B4,
            8 => Self::B8,
            other => Self::Part(other as u8),
        }
    }
}

/// Payload carried in a `Write` operation. Inline storage for sub-line writes;
/// boxed slice for line-sized writes.
#[derive(Clone, Debug)]
pub enum WriteData {
    /// Up to 8 bytes packed into a `u64` (low-order bytes used per `AccessSize`).
    Small(u64),
    /// A cache line's bytes, of which those whose bit is set in `mask` are
    /// written.
    Line {
        /// The line's bytes.
        bytes: Box<[u8]>,
        /// Which bytes the write covers, bit `i` for byte `i`.
        mask: u64,
    },
}

/// Response data payload for a load.
#[derive(Clone, Debug)]
pub enum MemRespData {
    /// Small inline payload (up to 8 bytes).
    Small(u64),
    /// What a hart's access read when the memory system served it: a
    /// load's bytes, an AMO's old value, or a store-conditional's result.
    Performed {
        /// The value, zero-extended.
        value: u64,
        /// Position in the order of RAM writes the value reflects; kept only
        /// when more than one hart can write.
        observed: Option<WriteSeq>,
    },
    /// A full cache line.
    Line(Box<[u8]>),
    /// What a hart's [`AccessSize::Span`] read when the memory system
    /// served it.
    PerformedBytes {
        /// The bytes, in address order.
        bytes: Box<[u8]>,
        /// Position in the order of RAM writes the bytes reflect; kept only
        /// when more than one hart can write.
        observed: Option<WriteSeq>,
    },
}

/// Whose write a [`MemOp::Write`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOrigin {
    /// A hart's store: its bytes land in RAM where the memory system serves
    /// it.
    Hart(HartId),
    /// A write whose originator has already put its bytes in RAM (a device's
    /// DMA, the page-table walker's A/D update); the packet carries only its
    /// timing.
    Placed,
    /// The simulator host writing a device register.
    Host,
}

/// Memory operation kind on a `MemReq`.
#[derive(Clone, Debug)]
pub enum MemOp {
    /// Demand or speculative load.
    Read,
    /// Read for ownership: a cache fetching a line it is about to write, so
    /// the responder grants `Modified`. Memory and devices treat it as a
    /// read.
    ReadOwn,
    /// Store with payload.
    Write {
        /// Bytes to write.
        data: WriteData,
        /// Whose write it is, which decides whether serving it writes RAM.
        origin: WriteOrigin,
    },
    /// Atomic read-modify-write.
    Atomic {
        /// The AMO sub-operation.
        op: AtomicOp,
        /// Source-register value for the AMO.
        data: u64,
        /// The hart performing it.
        hart: HartId,
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
    /// A cache-maintenance operation on the request's line (`cbo.clean`,
    /// `cbo.flush`, `cbo.inval`), gem5's `CleanSharedReq` /
    /// `CleanInvalidReq` / `InvalidateReq` to the point of coherence: every
    /// cache on the way applies it to its copy and passes it on, and
    /// memory acknowledges it.
    Maintain {
        /// What to do to the line.
        op: Maintenance,
        /// A cache it passed held the line modified: that data travels with
        /// it to memory, as gem5's `WriteClean`.
        dirty: bool,
    },
    /// A hardware prefetch of the request's line into the cache at `into`.
    /// Caches above `into` pass it down; the cache at `into` fetches the
    /// line unless it holds it, is already fetching it or is short of
    /// MSHRs. Nothing answers it.
    Prefetch {
        /// The level that fills the line.
        into: CacheLevel,
        /// Fetch the line with write permission, for a store stream.
        exclusive: bool,
    },
}

/// What a cache-maintenance operation does to every cached copy of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Maintenance {
    /// Write modified data back, keeping a clean copy (`cbo.clean`).
    Clean,
    /// Write modified data back and drop every copy (`cbo.flush`).
    Flush,
    /// Drop every copy, discarding modified data (`cbo.inval`).
    Invalidate,
}

impl MemOp {
    /// True for an access that takes effect where the memory system serves
    /// it, as gem5's cache satisfies a request: at the first cache holding
    /// the line with the permission the access needs, or at the memory
    /// controller when no cache does. Line fills and writebacks between
    /// levels only move permission and timing.
    #[must_use]
    pub const fn takes_effect_when_served(&self, size: AccessSize) -> bool {
        match self {
            Self::Read => !matches!(size, AccessSize::Line),
            Self::Atomic { .. } | Self::Write { origin: WriteOrigin::Hart(_), .. } => true,
            Self::ReadOwn
            | Self::Write { .. }
            | Self::Fetch
            | Self::Writeback { .. }
            | Self::Maintain { .. }
            | Self::Prefetch { .. } => false,
        }
    }
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

/// What a [`Packet::Probe`] asks of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeKind {
    /// Drop the line (a writer elsewhere wants it, or it is being recalled).
    Invalidate,
    /// Keep at most a shared copy (a reader elsewhere wants it).
    Downgrade,
    /// Keep the copy, clean (a `cbo.clean` elsewhere).
    Clean,
}

/// MESI / MOESI coherence state.
///
/// `Owned` is set only by MOESI implementations; MESI never produces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MesiState {
    /// Line is dirty and held exclusively here.
    Modified,
    /// Line is clean and held exclusively here.
    Exclusive,
    /// Line is clean; may be held in other caches too.
    Shared,
    /// Line is not held.
    #[default]
    Invalid,
}

/// DRAM command kind, carried by [`Packet::DramCmd`].
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
        /// PC of the instruction the request serves: a demand load's or a
        /// fetch's. Prefetchers that learn per instruction train on it.
        pc: Option<VirtAddr>,
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
        /// Coherence state the responder grants the requester for the line
        /// (`Shared` when another cache keeps a copy; memory and devices
        /// grant `Exclusive`). A write request is always granted `Modified`.
        state: MesiState,
    },
    /// A lower private cache asks an upper one to give up rights to a line
    /// on behalf of a snoop; answered with [`Packet::ProbeResp`].
    Probe {
        /// Line probed.
        line_addr: LineAddr,
        /// Whether the line must be dropped or may be kept uncore.
        kind: ProbeKind,
        /// Correlator the prober uses to collect the responses.
        txn: ReqId,
    },
    /// Answer to a [`Packet::Probe`]: the line has been dropped or
    /// downgraded (its dirty data, if any, was written back first).
    ProbeResp {
        /// Correlator from the probe.
        txn: ReqId,
        /// Whether the responder (or a cache above it) held the line at all.
        had_copy: bool,
        /// Whether the responder held the line modified.
        dirty: bool,
    },
    /// Invalidate a cache line (back-invalidation, FENCE.VMA, coherence-driven).
    CacheInval {
        /// Line to invalidate.
        line_addr: LineAddr,
    },
    /// A coherence message between a private L2 and the home agent.
    Coh(CoherenceMsg),
    /// Write back every dirty line of a data cache and invalidate every
    /// line, then answer with a `MemResp`: what FENCE.I needs when no cache
    /// below keeps instruction fetches coherent with the data cache.
    FlushAll {
        /// Requester's correlator.
        req_id: ReqId,
    },
    /// A cache's own reminder to look at the next line of its flush.
    FlushStep,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writedata_small_round_trip() {
        let w = WriteData::Small(0x12_34_56_78);
        match w {
            WriteData::Small(v) => assert_eq!(v, 0x12_34_56_78),
            WriteData::Line { .. } => panic!("wrong variant"),
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
