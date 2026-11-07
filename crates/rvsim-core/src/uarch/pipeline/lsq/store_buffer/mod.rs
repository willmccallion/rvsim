//! Store Buffer for deferred memory writes.
//!
//! Stores are not written to memory until they commit from the ROB. The store
//! buffer holds pending stores and provides:
//! 1. **Allocation:** Reserve a slot when a store enters the backend.
//! 2. **Resolution:** Fill in the physical address after Memory1 and the
//!    data when it is ready; a plain store's data may arrive before or after
//!    its address.
//! 3. **Forwarding:** Provide store-to-load forwarding for loads that hit a pending store.
//! 4. **Commit:** Mark entries as committed when the ROB retires the store.
//! 5. **Drain:** Send committed stores' writes to memory one per cycle, in
//!    order. An entry keeps its slot until the memory system acknowledges
//!    its write, and slots are released from the head, as gem5's store queue
//!    does: a store that misses in the cache holds the buffer until its line
//!    arrives.

use std::collections::VecDeque;

use crate::common::{PhysAddr, VirtAddr};
use crate::exec::cbo::CboEffect;
use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use crate::isa::op::MemWidth;
use crate::sim::components::ReqId;
use crate::uarch::pipeline::rob::RobTag;
use crate::uarch::pipeline::squash::SquashScope;

/// A resolved store that writes some of a load's bytes, over `start..end`.
#[derive(Clone, Copy, Debug)]
struct OverlappingStore {
    rob_tag: RobTag,
    start: u64,
    end: u64,
    data: Option<StoreData>,
}

/// Result of store-to-load forwarding check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardResult {
    /// Store fully covers the load — use the forwarded data.
    Hit(u64),
    /// No overlap with any pending store — safe to read from memory.
    Miss,
    /// Partial overlap — must stall until the store drains to memory.
    Stall,
}

/// Resolution state of a store buffer entry, encoding lifecycle and data.
///
/// Combines the lifecycle state with the associated physical address and data,
/// making it impossible to read an address from an unresolved store, or to
/// commit a store whose data has not arrived.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StoreResolution {
    /// Allocated; neither address nor data resolved.
    #[default]
    Pending,
    /// The data arrived before the address.
    PendingWithData {
        /// Data to write.
        data: StoreData,
    },
    /// Address resolved, waiting for ROB commit; the data may still be on
    /// its way.
    Ready {
        /// Physical address of the store.
        paddr: PhysAddr,
        /// Data to write, once it has arrived.
        data: Option<StoreData>,
    },
    /// ROB has committed this store; it can be drained to memory.
    Committed {
        /// Physical address of the store.
        paddr: PhysAddr,
        /// Data to write.
        data: StoreData,
    },
}

/// What a store-buffer entry writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreData {
    /// The low bytes of a value, as many as the store's width.
    Bytes(u64),
    /// A cache-block operation on the whole block at the entry's address,
    /// which behaves as a store for ordering.
    Block(CboEffect),
}

impl StoreData {
    /// The bytes `[start, end)` an entry covers when it is at `paddr`
    /// with `width`; a store whose data has not arrived is a byte store.
    const fn span(data: Option<Self>, paddr: PhysAddr, width: MemWidth) -> (u64, u64) {
        let start = paddr.val();
        match data {
            Some(Self::Block(_)) => (start, start + CBOZ_BLOCK_SIZE),
            Some(Self::Bytes(_)) | None => (start, start + width.bytes()),
        }
    }
}

impl StoreResolution {
    /// Whether this entry has been committed (or cancelled) and is ready to drain.
    pub const fn is_committed(&self) -> bool {
        matches!(self, Self::Committed { .. })
    }

    /// Whether this entry is still pending (no address resolved).
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending | Self::PendingWithData { .. })
    }

    /// Returns the physical address if resolved (Ready or Committed).
    pub const fn paddr(&self) -> Option<PhysAddr> {
        match self {
            Self::Ready { paddr, .. } | Self::Committed { paddr, .. } => Some(*paddr),
            Self::Pending | Self::PendingWithData { .. } => None,
        }
    }

    /// The resolved address and the data, if it has arrived.
    const fn address(&self) -> Option<(PhysAddr, Option<StoreData>)> {
        match *self {
            Self::Ready { paddr, data } => Some((paddr, data)),
            Self::Committed { paddr, data } => Some((paddr, Some(data))),
            Self::Pending | Self::PendingWithData { .. } => None,
        }
    }
}

/// Where a committed store's write to memory stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WriteProgress {
    /// Not yet sent.
    #[default]
    Unsent,
    /// Sent; waiting for the acknowledgement of each listed request.
    InFlight([Option<ReqId>; 2]),
    /// Acknowledged (or needed no write); the slot frees once it is the
    /// oldest.
    Done,
}

/// A committed store taken for writing by [`StoreBuffer::begin_write`];
/// [`StoreBuffer::issue_write`] records the requests that carry it.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct PendingWrite {
    /// The store.
    pub rob_tag: RobTag,
    /// Its width.
    pub width: MemWidth,
    /// Where it writes.
    pub paddr: PhysAddr,
    /// What it writes.
    pub data: StoreData,
}

/// A single entry in the store buffer.
///
/// The scalar store buffer holds one entry per scalar store instruction.
/// Vector stores have their own dedicated buffer (`vec_store_buffer`) and
/// do not allocate SB slots.
#[derive(Clone, Debug, Default)]
pub struct StoreBufferEntry {
    /// ROB tag of the store instruction.
    pub rob_tag: RobTag,
    /// Virtual address of the store.
    pub vaddr: VirtAddr,
    /// Width of the store operation.
    pub width: MemWidth,
    /// Resolution state — encodes lifecycle, physical address, and data.
    pub resolution: StoreResolution,
    /// Progress of the write once committed.
    pub write: WriteProgress,
}

/// Store buffer — FIFO queue of pending stores.
#[derive(Debug)]
pub struct StoreBuffer {
    /// The stores, oldest first: allocated in rename order, so in ROB tag
    /// order, and removed only from either end or by `retain`.
    entries: VecDeque<StoreBufferEntry>,
    /// The most stores it holds.
    capacity: usize,
}

impl StoreBuffer {
    /// Creates a new store buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self { entries: VecDeque::with_capacity(capacity), capacity }
    }

    #[cfg(test)]
    /// Returns the number of occupied entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the store buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns true if any committed stores are waiting to drain to RAM.
    ///
    /// Unlike `is_empty()`, this ignores speculative (Pending/Ready) entries.
    /// Used by the SFENCE.VMA stall: the fence only needs to wait for
    /// committed stores to reach RAM — younger speculative entries will be
    /// squashed by the full pipeline flush after the fence commits.
    pub fn has_committed_stores(&self) -> bool {
        self.entries.iter().any(|entry| entry.resolution.is_committed())
    }

    /// Returns true if the store buffer is full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.entries.len() == self.capacity
    }

    /// Slots not holding a store.
    #[inline]
    pub fn free_slots(&self) -> usize {
        self.capacity - self.entries.len()
    }

    /// Allocates a slot for a new store. Returns false if the buffer is full.
    pub fn allocate(&mut self, rob_tag: RobTag, width: MemWidth) -> bool {
        if self.is_full() {
            return false;
        }
        self.entries.push_back(StoreBufferEntry {
            rob_tag,
            vaddr: VirtAddr::new(0),
            width,
            resolution: StoreResolution::Pending,
            write: WriteProgress::Unsent,
        });
        true
    }

    /// Resolves a store's address and data after memory translation.
    pub fn resolve(&mut self, rob_tag: RobTag, vaddr: VirtAddr, paddr: PhysAddr, data: u64) {
        self.resolve_as(rob_tag, vaddr, paddr, StoreData::Bytes(data));
    }

    /// Resolves a store's address after memory translation, keeping the
    /// data if it has already arrived.
    pub fn resolve_address(&mut self, rob_tag: RobTag, vaddr: VirtAddr, paddr: PhysAddr) {
        if let Some(entry) = self.find_by_tag_mut(rob_tag) {
            let data = match entry.resolution {
                StoreResolution::PendingWithData { data } => Some(data),
                _ => None,
            };
            entry.vaddr = vaddr;
            entry.resolution = StoreResolution::Ready { paddr, data };
        }
    }

    /// Records a store's data, which its data half delivers independently
    /// of its address.
    pub fn resolve_data(&mut self, rob_tag: RobTag, value: u64) {
        let Some(entry) = self.find_by_tag_mut(rob_tag) else { return };
        let data = StoreData::Bytes(value);
        entry.resolution = match entry.resolution {
            StoreResolution::Pending | StoreResolution::PendingWithData { .. } => {
                StoreResolution::PendingWithData { data }
            }
            StoreResolution::Ready { paddr, .. } => {
                StoreResolution::Ready { paddr, data: Some(data) }
            }
            committed @ StoreResolution::Committed { .. } => committed,
        };
    }

    /// False only for a store in the buffer whose data has not arrived; a
    /// store that is not in the buffer has nothing outstanding.
    #[must_use]
    pub fn has_data(&self, rob_tag: RobTag) -> bool {
        self.find_by_tag(rob_tag).is_none_or(|entry| {
            !matches!(
                entry.resolution,
                StoreResolution::Pending | StoreResolution::Ready { data: None, .. }
            )
        })
    }

    /// Resolves a cache-block operation on the block at `block` after
    /// memory translation.
    pub fn resolve_block(
        &mut self,
        rob_tag: RobTag,
        vaddr: VirtAddr,
        block: PhysAddr,
        effect: CboEffect,
    ) {
        self.resolve_as(rob_tag, vaddr, block, StoreData::Block(effect));
    }

    fn resolve_as(&mut self, rob_tag: RobTag, vaddr: VirtAddr, paddr: PhysAddr, data: StoreData) {
        if let Some(entry) = self.find_by_tag_mut(rob_tag) {
            entry.vaddr = vaddr;
            entry.resolution = StoreResolution::Ready { paddr, data: Some(data) };
        }
    }

    /// Marks a store as committed (the ROB has retired the instruction).
    pub fn mark_committed(&mut self, rob_tag: RobTag) {
        let Some(entry) = self.find_by_tag_mut(rob_tag) else { return };
        debug_assert!(
            matches!(entry.resolution, StoreResolution::Ready { data: Some(_), .. }),
            "mark_committed on non-Ready entry: rob_tag={} resolution={:?}",
            rob_tag,
            entry.resolution,
        );
        if let StoreResolution::Ready { paddr, data: Some(data) } = entry.resolution {
            entry.resolution = StoreResolution::Committed { paddr, data };
        }
    }

    /// Attempts store-to-load forwarding from the youngest store older than
    /// the load that writes any of its bytes.
    ///
    /// Returns `Hit(data)` if that store fully covers the load, `Stall` if
    /// it overlaps only part of it or its data has not arrived (the load
    /// must wait for it to drain), or `Miss` if no older store overlaps.
    /// Stores not strictly older than `load_rob_tag` come after the load in
    /// program order and are skipped.
    pub fn forward_load(
        &self,
        paddr: PhysAddr,
        width: MemWidth,
        load_rob_tag: RobTag,
    ) -> ForwardResult {
        let Some(store) = self.youngest_older_overlap(paddr, width, load_rob_tag) else {
            return ForwardResult::Miss;
        };
        let load_size = width.bytes() as usize;
        let load_start = paddr.val();
        let load_end = load_start + load_size as u64;
        let covers = store.start <= load_start && store.end >= load_end;
        match store.data {
            Some(StoreData::Bytes(value)) if covers => {
                let offset = (load_start - store.start) as u32;
                let shifted = value >> (offset * 8);
                let mask = if load_size >= 8 { u64::MAX } else { (1u64 << (load_size * 8)) - 1 };
                ForwardResult::Hit(shifted & mask)
            }
            Some(StoreData::Block(CboEffect::Zero)) if covers => ForwardResult::Hit(0),
            Some(StoreData::Bytes(_) | StoreData::Block(_)) | None => ForwardResult::Stall,
        }
    }

    /// The youngest store older than `load_rob_tag` whose resolved address
    /// writes any byte of the load; [`Self::forward_load`] answers from it.
    #[must_use]
    pub fn youngest_older_store_to(
        &self,
        paddr: PhysAddr,
        width: MemWidth,
        load_rob_tag: RobTag,
    ) -> Option<RobTag> {
        self.youngest_older_overlap(paddr, width, load_rob_tag).map(|store| store.rob_tag)
    }

    fn youngest_older_overlap(
        &self,
        paddr: PhysAddr,
        width: MemWidth,
        load_rob_tag: RobTag,
    ) -> Option<OverlappingStore> {
        let load_start = paddr.val();
        let load_end = load_start + width.bytes();
        self.older_than(load_rob_tag).rev().find_map(|entry| {
            let (store_paddr, data) = entry.resolution.address()?;
            let (start, end) = StoreData::span(data, store_paddr, entry.width);
            (load_start < end && load_end > start).then_some(OverlappingStore {
                rob_tag: entry.rob_tag,
                start,
                end,
                data,
            })
        })
    }

    /// The stores older than `rob_tag`, oldest first.
    fn older_than(&self, rob_tag: RobTag) -> impl DoubleEndedIterator<Item = &StoreBufferEntry> {
        let older = self.entries.partition_point(|entry| entry.rob_tag.is_older_than(rob_tag));
        self.entries.range(..older)
    }

    /// True when a resolved store older than `load_rob_tag` writes any of
    /// the `bytes` bytes at `paddr`. A vector access, which no scalar store
    /// can supply whole, waits for such a store to be written.
    #[must_use]
    pub fn overlaps_older_store(
        &self,
        paddr: PhysAddr,
        bytes: usize,
        load_rob_tag: RobTag,
    ) -> bool {
        let load_start = paddr.val();
        let load_end = load_start + bytes as u64;
        self.older_than(load_rob_tag).any(|entry| {
            entry.resolution.address().is_some_and(|(store_paddr, data)| {
                let (store_start, store_end) = StoreData::span(data, store_paddr, entry.width);
                load_start < store_end && load_end > store_start
            })
        })
    }

    /// Checks whether any store buffer entry older than `rob_tag` has an
    /// unresolved address. Used by the issue queue to prevent loads from
    /// issuing before older stores have their addresses resolved.
    pub fn has_unresolved_store_before(&self, rob_tag: RobTag) -> bool {
        self.older_than(rob_tag).any(|entry| entry.resolution.is_pending())
    }

    /// Checks whether a specific store is unresolved (no address yet).
    ///
    /// Returns `true` if the store is found in the buffer and still has no
    /// resolved address (Pending state). Returns `false` if the store is
    /// resolved, committed, or not found (stale tag).
    pub fn is_unresolved(&self, rob_tag: RobTag) -> bool {
        self.find_by_tag(rob_tag).is_some_and(|entry| entry.resolution.is_pending())
    }

    /// Checks whether any store buffer entry older than `rob_tag` overlaps
    /// the given physical address range. Used by LR/AMO to stall until older
    /// stores to the same address have drained, preserving atomicity.
    pub fn has_older_store_to(&self, paddr: PhysAddr, width: MemWidth, rob_tag: RobTag) -> bool {
        let load_start = paddr.val();
        let load_end = load_start + width.bytes();
        self.older_than(rob_tag).any(|entry| {
            // An unresolved store to an unknown address may overlap.
            entry.resolution.address().is_none_or(|(store_paddr, data)| {
                let (store_start, store_end) = StoreData::span(data, store_paddr, entry.width);
                load_start < store_end && load_end > store_start
            })
        })
    }

    /// Takes the oldest committed store whose write has not been sent, in
    /// program order. `None` when there is none.
    pub fn begin_write(&self) -> Option<PendingWrite> {
        for entry in &self.entries {
            let StoreResolution::Committed { paddr, data } = entry.resolution else {
                return None;
            };
            if entry.write == WriteProgress::Unsent {
                return Some(PendingWrite {
                    rob_tag: entry.rob_tag,
                    width: entry.width,
                    paddr,
                    data,
                });
            }
        }
        None
    }

    /// Records the requests carrying `write`; with none (the store needed no
    /// write) it is done at once.
    pub fn issue_write(&mut self, write: PendingWrite, requests: &[ReqId]) {
        let entry = self.find_by_tag_mut(write.rob_tag);
        debug_assert!(entry.is_some(), "stale pending write");
        let Some(entry) = entry else { return };
        let mut outstanding = [None; 2];
        for (slot, req) in outstanding.iter_mut().zip(requests) {
            *slot = Some(*req);
        }
        debug_assert!(requests.len() <= outstanding.len(), "a store is at most two requests");
        entry.write = if requests.is_empty() {
            WriteProgress::Done
        } else {
            WriteProgress::InFlight(outstanding)
        };
        self.release_done();
    }

    /// The memory system acknowledged `req`. Returns whether it belonged to
    /// a store in this buffer.
    pub fn write_acked(&mut self, req: ReqId) -> bool {
        let Some(entry) = self.entries.iter_mut().find(
            |e| matches!(e.write, WriteProgress::InFlight(reqs) if reqs.contains(&Some(req))),
        ) else {
            return false;
        };
        let WriteProgress::InFlight(mut reqs) = entry.write else { return false };
        for slot in &mut reqs {
            if *slot == Some(req) {
                *slot = None;
            }
        }
        entry.write = if reqs.iter().all(Option::is_none) {
            WriteProgress::Done
        } else {
            WriteProgress::InFlight(reqs)
        };
        self.release_done();
        true
    }

    /// Frees the slots of the oldest stores whose writes are done.
    fn release_done(&mut self) {
        while self.entries.front().is_some_and(|head| head.write == WriteProgress::Done) {
            let _ = self.entries.pop_front();
        }
    }

    /// Removes the uncommitted stores `scope` squashes; committed stores
    /// remain to be written.
    pub fn squash(&mut self, scope: SquashScope) {
        match scope {
            SquashScope::After(keep) => self.flush_after(keep),
            SquashScope::All => self.flush_speculative(),
        }
    }

    /// Flushes speculative (non-committed) entries. Committed entries remain.
    pub fn flush_speculative(&mut self) {
        self.entries.retain(|entry| entry.resolution.is_committed());
    }

    /// Flushes store buffer entries allocated *after* the given ROB tag.
    ///
    /// Entries with tags up to and including `keep_tag` are retained (whether
    /// Pending, Ready, or Committed). Only entries whose ROB tag is strictly
    /// newer than `keep_tag` are removed.
    ///
    /// This is used on branch mispredictions where pre-branch stores that are
    /// still in-flight (Ready but not yet Committed) must be kept.
    pub fn flush_after(&mut self, keep_tag: RobTag) {
        self.entries.retain(|entry| entry.rob_tag.is_older_or_eq(keep_tag));
    }

    #[cfg(test)]
    /// Flushes all entries (including committed ones).
    pub fn flush_all(&mut self) {
        self.entries.clear();
    }

    /// Frees the slot of an AMO or store-conditional the cache has
    /// performed, which has nothing left to write. Having waited for every
    /// older store, it is the oldest entry.
    pub fn remove_performed(&mut self, rob_tag: RobTag) {
        let is_head = self.entries.front().is_some_and(|head| head.rob_tag == rob_tag);
        debug_assert!(is_head, "a performed atomic is the oldest store");
        if is_head {
            let _ = self.entries.pop_front();
        }
    }

    /// Returns the resolved physical address for the entry with the given ROB tag,
    /// or `None` if the entry is not found or has no address yet.
    pub fn find_paddr(&self, rob_tag: RobTag) -> Option<PhysAddr> {
        self.find_by_tag(rob_tag)?.resolution.paddr()
    }

    /// The write a committed plain store makes, for the commit log; `None`
    /// for a cache-block operation or a tag the buffer does not hold.
    #[cfg(feature = "commit-log")]
    pub fn committed_write(
        &self,
        rob_tag: RobTag,
    ) -> Option<crate::uarch::pipeline::commit_log::MemEffect> {
        let entry = self.find_by_tag(rob_tag)?;
        let StoreResolution::Committed { paddr, data: StoreData::Bytes(data) } = entry.resolution
        else {
            return None;
        };
        Some(crate::uarch::pipeline::commit_log::MemEffect::new(
            entry.vaddr,
            paddr,
            entry.width,
            None,
            Some(data),
        ))
    }

    fn position(&self, rob_tag: RobTag) -> Option<usize> {
        self.entries.binary_search_by(|entry| entry.rob_tag.age_cmp(rob_tag)).ok()
    }

    fn find_by_tag(&self, rob_tag: RobTag) -> Option<&StoreBufferEntry> {
        self.entries.get(self.position(rob_tag)?)
    }

    fn find_by_tag_mut(&mut self, rob_tag: RobTag) -> Option<&mut StoreBufferEntry> {
        let position = self.position(rob_tag)?;
        self.entries.get_mut(position)
    }
}

#[cfg(test)]
mod tests;
