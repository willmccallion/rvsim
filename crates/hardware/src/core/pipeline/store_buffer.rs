//! Store Buffer for deferred memory writes.
//!
//! Stores are not written to memory until they commit from the ROB. The store
//! buffer holds pending stores and provides:
//! 1. **Allocation:** Reserve a slot when a store enters the backend.
//! 2. **Resolution:** Fill in the physical address and data after Memory1/Memory2.
//! 3. **Forwarding:** Provide store-to-load forwarding for loads that hit a pending store.
//! 4. **Commit:** Mark entries as committed when the ROB retires the store.
//! 5. **Drain:** Send committed stores' writes to memory one per cycle, in
//!    order. An entry keeps its slot until the memory system acknowledges
//!    its write, and slots are released from the head, as gem5's store queue
//!    does: a store that misses in the cache holds the buffer until its line
//!    arrives.

use crate::common::{PhysAddr, VirtAddr};
use crate::core::pipeline::rob::RobTag;
use crate::exec::cbo::CboEffect;
use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use crate::isa::op::MemWidth;
use crate::sim::components::ReqId;

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
/// making it impossible to read address/data from an unresolved store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StoreResolution {
    /// Allocated but address/data not yet resolved.
    #[default]
    Pending,
    /// Address and data resolved, waiting for ROB commit.
    Ready {
        /// Physical address of the store.
        paddr: PhysAddr,
        /// Data to write.
        data: StoreData,
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
    /// The bytes `[start, end)` the entry covers when it is at `paddr`
    /// with `width`.
    const fn span(self, paddr: PhysAddr, width: MemWidth) -> (u64, u64) {
        let start = paddr.val();
        match self {
            Self::Bytes(_) => (start, start + width_to_bytes(width) as u64),
            Self::Block(_) => (start, start + CBOZ_BLOCK_SIZE),
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
        matches!(self, Self::Pending)
    }

    /// Returns the physical address if resolved (Ready or Committed).
    pub const fn paddr(&self) -> Option<PhysAddr> {
        match self {
            Self::Ready { paddr, .. } | Self::Committed { paddr, .. } => Some(*paddr),
            Self::Pending => None,
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
    slot: usize,
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
    /// Whether this slot is occupied.
    pub valid: bool,
}

/// Store buffer — FIFO queue of pending stores.
#[derive(Debug)]
pub struct StoreBuffer {
    entries: Vec<StoreBufferEntry>,
    /// Index of the oldest entry.
    head: usize,
    /// Index where the next entry will be allocated.
    tail: usize,
    /// Number of valid entries.
    count: usize,
}

impl StoreBuffer {
    /// Creates a new store buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let mut entries = Vec::with_capacity(capacity);
        entries.resize_with(capacity, StoreBufferEntry::default);
        Self { entries, head: 0, tail: 0, count: 0 }
    }

    /// Returns the capacity.
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Returns the number of occupied entries.
    #[inline]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Returns true if the store buffer is empty.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns true if any committed stores are waiting to drain to RAM.
    ///
    /// Unlike `is_empty()`, this ignores speculative (Pending/Ready) entries.
    /// Used by the SFENCE.VMA stall: the fence only needs to wait for
    /// committed stores to reach RAM — younger speculative entries will be
    /// squashed by the full pipeline flush after the fence commits.
    pub fn has_committed_stores(&self) -> bool {
        if self.count == 0 {
            return false;
        }
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.resolution.is_committed() {
                return true;
            }
            idx = (idx + 1) % cap;
        }
        false
    }

    /// Returns true if the store buffer is full.
    #[inline]
    pub const fn is_full(&self) -> bool {
        self.count == self.entries.len()
    }

    /// Returns the number of free slots.
    #[inline]
    pub const fn free_slots(&self) -> usize {
        self.entries.len() - self.count
    }

    /// Allocates a slot for a new store. Returns false if the buffer is full.
    pub fn allocate(&mut self, rob_tag: RobTag, width: MemWidth) -> bool {
        if self.is_full() {
            return false;
        }

        self.entries[self.tail] = StoreBufferEntry {
            rob_tag,
            vaddr: VirtAddr::new(0),
            width,
            resolution: StoreResolution::Pending,
            write: WriteProgress::Unsent,
            valid: true,
        };

        self.tail = (self.tail + 1) % self.entries.len();
        self.count += 1;
        true
    }

    /// Resolves a store's address and data after memory translation.
    pub fn resolve(&mut self, rob_tag: RobTag, vaddr: VirtAddr, paddr: PhysAddr, data: u64) {
        self.resolve_as(rob_tag, vaddr, paddr, StoreData::Bytes(data));
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
            entry.resolution = StoreResolution::Ready { paddr, data };
        }
    }

    /// Marks a store as committed (the ROB has retired the instruction).
    pub fn mark_committed(&mut self, rob_tag: RobTag) {
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &mut self.entries[idx];
            if entry.valid && entry.rob_tag == rob_tag {
                debug_assert!(
                    matches!(entry.resolution, StoreResolution::Ready { .. }),
                    "mark_committed on non-Ready entry: rob_tag={} resolution={:?}",
                    rob_tag.0,
                    entry.resolution,
                );
                if let StoreResolution::Ready { paddr, data } = entry.resolution {
                    entry.resolution = StoreResolution::Committed { paddr, data };
                }
            }
            idx = (idx + 1) % cap;
        }
    }

    /// Attempts store-to-load forwarding.
    ///
    /// Returns `Hit(data)` if a pending store fully covers the load,
    /// `Stall` if a store partially overlaps (must wait for drain),
    /// or `Miss` if no overlap exists.
    ///
    /// `load_rob_tag` is the ROB tag of the load instruction. Only stores
    /// older than the load (lower tag) are considered for forwarding. Stores
    /// newer than the load in program order are skipped.
    pub fn forward_load(
        &self,
        paddr: PhysAddr,
        width: MemWidth,
        load_rob_tag: RobTag,
    ) -> ForwardResult {
        let load_size = width_to_bytes(width);
        let load_start = paddr.val();
        let load_end = load_start + load_size as u64;

        let mut idx = if self.tail == 0 { self.entries.len() - 1 } else { self.tail - 1 };

        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid {
                // Skip stores not strictly older than the load: they come after in program order.
                if !entry.rob_tag.is_older_than(load_rob_tag) {
                    if idx == 0 {
                        idx = self.entries.len() - 1;
                    } else {
                        idx -= 1;
                    }
                    continue;
                }

                if let StoreResolution::Ready { paddr: store_paddr, data }
                | StoreResolution::Committed { paddr: store_paddr, data } = entry.resolution
                {
                    let (store_start, store_end) = data.span(store_paddr, entry.width);
                    if load_start < store_end && load_end > store_start {
                        let covers = store_start <= load_start && store_end >= load_end;
                        return match data {
                            StoreData::Bytes(value) if covers => {
                                let offset = (load_start - store_start) as u32;
                                let shifted = value >> (offset * 8);
                                let mask = if load_size >= 8 {
                                    u64::MAX
                                } else {
                                    (1u64 << (load_size * 8)) - 1
                                };
                                ForwardResult::Hit(shifted & mask)
                            }
                            StoreData::Block(CboEffect::Zero) if covers => ForwardResult::Hit(0),
                            StoreData::Bytes(_) | StoreData::Block(_) => ForwardResult::Stall,
                        };
                    }
                }
            }
            if idx == 0 {
                idx = self.entries.len() - 1;
            } else {
                idx -= 1;
            }
        }

        ForwardResult::Miss
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
        self.entries.iter().any(|entry| {
            if !entry.valid || !entry.rob_tag.is_older_than(load_rob_tag) {
                return false;
            }
            match entry.resolution {
                StoreResolution::Pending => false,
                StoreResolution::Ready { paddr: store_paddr, data }
                | StoreResolution::Committed { paddr: store_paddr, data } => {
                    let (store_start, store_end) = data.span(store_paddr, entry.width);
                    load_start < store_end && load_end > store_start
                }
            }
        })
    }

    /// Checks whether any store buffer entry older than `rob_tag` has an
    /// unresolved address. Used by the issue queue to prevent loads from
    /// issuing before older stores have their addresses resolved.
    pub fn has_unresolved_store_before(&self, rob_tag: RobTag) -> bool {
        if self.count == 0 {
            return false;
        }
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.rob_tag.is_older_than(rob_tag) && entry.resolution.is_pending()
            {
                return true;
            }
            idx = (idx + 1) % cap;
        }
        false
    }

    /// Checks whether a specific store is unresolved (no address yet).
    ///
    /// Returns `true` if the store is found in the buffer and still has no
    /// resolved address (Pending state). Returns `false` if the store is
    /// resolved, committed, or not found (stale tag).
    pub fn is_unresolved(&self, rob_tag: RobTag) -> bool {
        if self.count == 0 {
            return false;
        }
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.rob_tag == rob_tag {
                return entry.resolution.is_pending();
            }
            idx = (idx + 1) % cap;
        }
        false
    }

    /// Checks whether any store buffer entry older than `rob_tag` overlaps
    /// the given physical address range. Used by LR/AMO to stall until older
    /// stores to the same address have drained, preserving atomicity.
    pub fn has_older_store_to(&self, paddr: PhysAddr, width: MemWidth, rob_tag: RobTag) -> bool {
        if self.count == 0 {
            return false;
        }
        let load_size = width_to_bytes(width) as u64;
        let load_start = paddr.val();
        let load_end = load_start + load_size;
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.rob_tag.is_older_than(rob_tag) {
                match entry.resolution {
                    // Unresolved store to unknown address — assume overlap.
                    StoreResolution::Pending => return true,
                    StoreResolution::Ready { paddr: store_paddr, data }
                    | StoreResolution::Committed { paddr: store_paddr, data } => {
                        let (store_start, store_end) = data.span(store_paddr, entry.width);
                        if load_start < store_end && load_end > store_start {
                            return true;
                        }
                    }
                }
            }
            idx = (idx + 1) % cap;
        }
        false
    }

    /// Takes the oldest committed store whose write has not been sent, in
    /// program order. `None` when there is none.
    pub fn begin_write(&mut self) -> Option<PendingWrite> {
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if !entry.valid {
                return None;
            }
            let StoreResolution::Committed { paddr, data } = entry.resolution else {
                return None;
            };
            if entry.write == WriteProgress::Unsent {
                return Some(PendingWrite {
                    slot: idx,
                    rob_tag: entry.rob_tag,
                    width: entry.width,
                    paddr,
                    data,
                });
            }
            idx = (idx + 1) % cap;
        }
        None
    }

    /// Records the requests carrying `write`; with none (the store needed no
    /// write) it is done at once.
    pub fn issue_write(&mut self, write: PendingWrite, requests: &[ReqId]) {
        let entry = &mut self.entries[write.slot];
        debug_assert!(entry.valid && entry.rob_tag == write.rob_tag, "stale pending write");
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
        let Some(entry) = self.entries.iter_mut().find(|e| {
            e.valid && matches!(e.write, WriteProgress::InFlight(reqs) if reqs.contains(&Some(req)))
        }) else {
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
        while self.count > 0 {
            let head = &mut self.entries[self.head];
            if !head.valid || head.write != WriteProgress::Done {
                return;
            }
            head.valid = false;
            self.head = (self.head + 1) % self.entries.len();
            self.count -= 1;
        }
    }

    /// Flushes speculative (non-committed) entries. Committed entries remain.
    pub fn flush_speculative(&mut self) {
        if self.count == 0 {
            return;
        }

        let cap = self.entries.len();
        let mut new_tail = self.head;
        let mut new_count = 0;
        let mut idx = self.head;

        for _ in 0..self.count {
            if self.entries[idx].valid && self.entries[idx].resolution.is_committed() {
                if idx != new_tail {
                    self.entries[new_tail] = self.entries[idx].clone();
                    self.entries[idx].valid = false;
                }
                new_tail = (new_tail + 1) % cap;
                new_count += 1;
            } else {
                self.entries[idx].valid = false;
            }
            idx = (idx + 1) % cap;
        }

        self.tail = new_tail;
        self.count = new_count;
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
        if self.count == 0 {
            return;
        }

        let cap = self.entries.len();
        let mut new_tail = self.head;
        let mut new_count = 0;
        let mut idx = self.head;

        for _ in 0..self.count {
            let entry = &self.entries[idx];
            if entry.valid && entry.rob_tag.is_older_or_eq(keep_tag) {
                if idx != new_tail {
                    self.entries[new_tail] = self.entries[idx].clone();
                    self.entries[idx].valid = false;
                }
                new_tail = (new_tail + 1) % cap;
                new_count += 1;
            } else {
                self.entries[idx].valid = false;
            }
            idx = (idx + 1) % cap;
        }

        self.tail = new_tail;
        self.count = new_count;
    }

    /// Flushes all entries (including committed ones).
    pub fn flush_all(&mut self) {
        for entry in &mut self.entries {
            entry.valid = false;
        }
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }

    /// Frees the slot of an AMO or store-conditional the cache has
    /// performed, which has nothing left to write. Having waited for every
    /// older store, it is the oldest entry.
    pub fn remove_performed(&mut self, rob_tag: RobTag) {
        let head = &mut self.entries[self.head];
        debug_assert!(
            self.count > 0 && head.valid && head.rob_tag == rob_tag,
            "a performed atomic is the oldest store"
        );
        if self.count > 0 && head.valid && head.rob_tag == rob_tag {
            head.valid = false;
            self.head = (self.head + 1) % self.entries.len();
            self.count -= 1;
        }
    }

    /// Returns the resolved physical address for the entry with the given ROB tag,
    /// or `None` if the entry is not found or has no address yet.
    pub fn find_paddr(&self, rob_tag: RobTag) -> Option<PhysAddr> {
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            if self.entries[idx].valid && self.entries[idx].rob_tag == rob_tag {
                return self.entries[idx].resolution.paddr();
            }
            idx = (idx + 1) % cap;
        }
        None
    }

    fn find_by_tag_mut(&mut self, rob_tag: RobTag) -> Option<&mut StoreBufferEntry> {
        let cap = self.entries.len();
        let mut idx = self.head;
        for _ in 0..self.count {
            if self.entries[idx].valid && self.entries[idx].rob_tag == rob_tag {
                return Some(&mut self.entries[idx]);
            }
            idx = (idx + 1) % cap;
        }
        None
    }
}

/// Converts a `MemWidth` to byte count.
pub(crate) const fn width_to_bytes(w: MemWidth) -> usize {
    match w {
        MemWidth::Byte => 1,
        MemWidth::Half => 2,
        MemWidth::Word => 4,
        MemWidth::Double => 8,
        MemWidth::Nop => 0,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;

    /// Sends the next write and completes it at once, returning what was
    /// written.
    fn drain_now(sb: &mut StoreBuffer) -> Option<StoreResolution> {
        let write = sb.begin_write()?;
        let resolution = StoreResolution::Committed { paddr: write.paddr, data: write.data };
        sb.issue_write(write, &[]);
        Some(resolution)
    }

    fn committed_store(sb: &mut StoreBuffer, tag: u32, paddr: u64, data: u64) {
        assert!(sb.allocate(RobTag(tag), MemWidth::Double));
        sb.resolve(RobTag(tag), VirtAddr::new(paddr), PhysAddr::new(paddr), data);
        sb.mark_committed(RobTag(tag));
    }

    #[test]
    fn a_sent_store_keeps_its_slot_and_forwards_until_acknowledged() {
        let mut sb = StoreBuffer::new(4);
        committed_store(&mut sb, 1, 0x1000, 0x55);
        let write = sb.begin_write().expect("committed store");
        sb.issue_write(write, &[ReqId::new(7)]);

        let before_ack = (sb.len(), sb.has_committed_stores());
        let forwarded = sb.forward_load(PhysAddr::new(0x1000), MemWidth::Double, RobTag(2));
        let known = sb.write_acked(ReqId::new(7));

        assert_eq!((before_ack, forwarded), ((1, true), ForwardResult::Hit(0x55)));
        assert!(known && sb.is_empty());
    }

    #[test]
    fn a_younger_store_is_sent_while_an_older_one_is_in_flight() {
        let mut sb = StoreBuffer::new(4);
        committed_store(&mut sb, 1, 0x1000, 1);
        committed_store(&mut sb, 2, 0x2000, 2);
        let first = sb.begin_write().expect("first store");
        sb.issue_write(first, &[ReqId::new(1)]);

        let second = sb.begin_write().expect("second store");

        assert_eq!(second.rob_tag, RobTag(2));
    }

    #[test]
    fn slots_free_in_order_when_a_younger_write_is_acknowledged_first() {
        let mut sb = StoreBuffer::new(4);
        committed_store(&mut sb, 1, 0x1000, 1);
        committed_store(&mut sb, 2, 0x2000, 2);
        let first = sb.begin_write().expect("first store");
        sb.issue_write(first, &[ReqId::new(1)]);
        let second = sb.begin_write().expect("second store");
        sb.issue_write(second, &[ReqId::new(2)]);

        let _ = sb.write_acked(ReqId::new(2));
        let after_younger = sb.len();
        let _ = sb.write_acked(ReqId::new(1));

        assert_eq!((after_younger, sb.len()), (2, 0));
    }

    #[test]
    fn a_store_written_as_two_requests_waits_for_both() {
        let mut sb = StoreBuffer::new(4);
        committed_store(&mut sb, 1, 0x103C, 1);
        let write = sb.begin_write().expect("committed store");
        sb.issue_write(write, &[ReqId::new(1), ReqId::new(2)]);

        let _ = sb.write_acked(ReqId::new(1));
        let after_one = sb.len();
        let _ = sb.write_acked(ReqId::new(2));

        assert_eq!((after_one, sb.len()), (1, 0));
    }

    #[test]
    fn test_allocate_and_drain() {
        let mut sb = StoreBuffer::new(4);
        assert!(sb.is_empty());

        let tag = RobTag(1);
        assert!(sb.allocate(tag, MemWidth::Word));
        assert_eq!(sb.len(), 1);

        // Can't drain yet (still Pending)
        assert!(sb.begin_write().is_none());

        sb.resolve(tag, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000), 0xDEADBEEF);
        // Can't drain yet (Ready but not Committed)
        assert!(sb.begin_write().is_none());

        sb.mark_committed(tag);
        let entry = drain_now(&mut sb).unwrap();
        assert_eq!(
            entry,
            StoreResolution::Committed {
                paddr: PhysAddr::new(0x8000_0000),
                data: StoreData::Bytes(0xDEADBEEF)
            }
        );
        assert!(sb.is_empty());
    }

    #[test]
    fn test_full_buffer() {
        let mut sb = StoreBuffer::new(2);
        assert!(sb.allocate(RobTag(1), MemWidth::Word));
        assert!(sb.allocate(RobTag(2), MemWidth::Word));
        assert!(sb.is_full());
        assert!(!sb.allocate(RobTag(3), MemWidth::Word));
    }

    #[test]
    fn test_forward_load() {
        let mut sb = StoreBuffer::new(4);
        let tag = RobTag(1);
        sb.allocate(tag, MemWidth::Word);
        sb.resolve(tag, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000), 0x12345678);

        // Forward should find the store (load is younger: tag 2 > store tag 1)
        let result = sb.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Word, RobTag(2));
        assert_eq!(result, ForwardResult::Hit(0x12345678));

        // Different address should miss
        let result = sb.forward_load(PhysAddr::new(0x8000_0004), MemWidth::Word, RobTag(2));
        assert_eq!(result, ForwardResult::Miss);
    }

    #[test]
    fn test_forward_load_byte() {
        let mut sb = StoreBuffer::new(4);
        let tag = RobTag(1);
        sb.allocate(tag, MemWidth::Word);
        sb.resolve(tag, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000), 0x12345678);

        // Forward a byte from the same address
        let result = sb.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag(2));
        assert_eq!(result, ForwardResult::Hit(0x78));
    }

    #[test]
    fn test_flush_speculative() {
        let mut sb = StoreBuffer::new(4);
        let t1 = RobTag(1);
        let t2 = RobTag(2);
        let t3 = RobTag(3);

        sb.allocate(t1, MemWidth::Word);
        sb.allocate(t2, MemWidth::Word);
        sb.allocate(t3, MemWidth::Word);

        sb.resolve(t1, VirtAddr::new(0x1000), PhysAddr::new(0x8000_0000), 10);
        sb.mark_committed(t1);

        sb.resolve(t2, VirtAddr::new(0x1004), PhysAddr::new(0x8000_0004), 20);
        // t2 is Ready but not committed
        // t3 is still Pending

        sb.flush_speculative();
        assert_eq!(sb.len(), 1); // only t1 remains

        let entry = drain_now(&mut sb).unwrap();
        assert_eq!(
            entry,
            StoreResolution::Committed {
                paddr: PhysAddr::new(0x8000_0000),
                data: StoreData::Bytes(10)
            }
        );
    }

    #[test]
    fn test_flush_all() {
        let mut sb = StoreBuffer::new(4);
        sb.allocate(RobTag(1), MemWidth::Word);
        sb.allocate(RobTag(2), MemWidth::Word);

        sb.flush_all();
        assert!(sb.is_empty());
    }

    #[test]
    fn test_circular_wraparound() {
        let mut sb = StoreBuffer::new(2);
        for i in 1..=10 {
            let tag = RobTag(i);
            sb.allocate(tag, MemWidth::Word);
            sb.resolve(tag, VirtAddr::new(0), PhysAddr::new(0x8000_0000), i as u64);
            sb.mark_committed(tag);
            let entry = drain_now(&mut sb).unwrap();
            assert_eq!(
                entry,
                StoreResolution::Committed {
                    paddr: PhysAddr::new(0x8000_0000),
                    data: StoreData::Bytes(i as u64)
                }
            );
        }
    }
}
