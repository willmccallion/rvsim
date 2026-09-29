//! Typed component identifiers for arena-indexed lookup, and a `ComponentId`
//! enum that names every addressable target on the event queue.
//!
//! Components (caches, memory controllers, pipelines, devices) live in flat
//! `Vec<T>` arenas on the [`Simulator`](crate::system::simulator::Simulator).
//! Cross-references travel as typed ID newtypes so the compiler refuses to
//! mix, say, a `CacheId` with a `MemCtrlId` even though both wrap `u32`.

/// Index of a cache (any level) in the `Simulator.caches` arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CacheId(u32);

/// Index of a pipeline in the `Simulator.pipelines` arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PipelineId(u32);

/// Index of a memory controller in the `Simulator.mem_ctrls` arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MemCtrlId(u32);

/// Index of a device in the `Simulator.devices` arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DeviceId(u32);

/// DRAM channel index inside a memory controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ChannelId(u8);

/// DDR5 sub-channel index inside a channel (0 or 1 for standard DDR5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SubchannelId(u8);

/// DRAM rank index inside a sub-channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RankId(u8);

/// DRAM bank-group index inside a rank.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BankGroupId(u8);

/// DRAM row index inside a bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct RowId(u32);

/// Per-packet request identifier; correlates a request with its response.
///
/// Originators (pipeline LSU, MSHR slots, prefetchers) assign a fresh `ReqId`
/// when injecting a request and match it against the `req_id` field of the
/// returning response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ReqId(u64);

macro_rules! impl_id_methods {
    ($id:ident, $raw:ty) => {
        impl $id {
            /// Creates a new identifier from a raw value.
            #[inline(always)]
            pub const fn new(val: $raw) -> Self {
                Self(val)
            }

            /// Returns the raw value.
            #[inline(always)]
            pub const fn val(self) -> $raw {
                self.0
            }

            /// Returns the value as a `usize` for use as a vector index.
            #[inline(always)]
            pub const fn as_index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

impl ReqId {
    /// A correlator for a request a cache originates itself (a line fetch
    /// or a writeback). The top bit marks the cache namespace so it can
    /// never collide with a pipeline's `(PipelineId << 48) | seq` ids.
    #[must_use]
    pub const fn for_cache(cache: CacheId, seq: u64) -> Self {
        Self((1 << 63) | ((cache.0 as u64) << 48) | (seq & ((1 << 48) - 1)))
    }
}

impl ReqId {
    /// A correlator for a transaction the coherence fabric originates
    /// itself (a recall, or a data fetch from the LLC on a requester's
    /// behalf); bit 62 marks the fabric namespace.
    #[must_use]
    pub const fn for_fabric(seq: u64) -> Self {
        Self((1 << 62) | (seq & ((1 << 48) - 1)))
    }

    /// A correlator for a DMA request a device originates; bit 61 marks
    /// the device namespace.
    #[must_use]
    pub const fn for_device(device: DeviceId, seq: u64) -> Self {
        Self((1 << 61) | ((device.0 as u64) << 48) | (seq & ((1 << 48) - 1)))
    }
}

impl_id_methods!(CacheId, u32);
impl_id_methods!(PipelineId, u32);
impl_id_methods!(MemCtrlId, u32);
impl_id_methods!(DeviceId, u32);
impl_id_methods!(ReqId, u64);
impl_id_methods!(ChannelId, u8);
impl_id_methods!(SubchannelId, u8);
impl_id_methods!(RankId, u8);
impl_id_methods!(BankGroupId, u8);
impl_id_methods!(RowId, u32);

use crate::common::{CoreId, HartId};

/// Addressable target on the event queue: any component that can receive packets.
///
/// `Bus` is currently singular; a multi-bus design would extend this with
/// `Bus(BusId)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComponentId {
    /// A hardware thread (architectural state).
    Hart(HartId),
    /// A physical core (caches, pipeline, branch predictor).
    Core(CoreId),
    /// A pipeline instance (one per core).
    Pipeline(PipelineId),
    /// A cache at any level.
    Cache(CacheId),
    /// The system bus.
    Bus,
    /// A memory controller (DDR / HBM / etc.).
    MemCtrl(MemCtrlId),
    /// An MMIO device (UART, CLINT, PLIC, …).
    Device(DeviceId),
    /// The coherence fabric: home agent plus interconnect.
    Fabric,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let c = CacheId::new(7);
        assert_eq!(c.val(), 7);
        assert_eq!(c.as_index(), 7);
        let p = PipelineId::new(2);
        let m = MemCtrlId::new(0);
        let d = DeviceId::new(3);
        let r = ReqId::new(0xDEAD_BEEF);
        assert_eq!(p.val(), 2);
        assert_eq!(m.val(), 0);
        assert_eq!(d.val(), 3);
        assert_eq!(r.val(), 0xDEAD_BEEF);
    }

    #[test]
    fn component_id_variants_are_distinct() {
        let a = ComponentId::Cache(CacheId::new(0));
        let b = ComponentId::MemCtrl(MemCtrlId::new(0));
        assert_ne!(a, b);
    }
}
