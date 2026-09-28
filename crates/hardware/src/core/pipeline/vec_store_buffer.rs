//! Vector Store Buffer (VSB) — dedicated holding structure for in-flight
//! vector stores in the O3 backend.
//!
//! ## Design
//!
//! One entry per in-flight vector store instruction, indexed by `RobTag`.
//! Each entry contains 64-byte cache-line buffers with byte-valid masks.
//! Element resolves at memory2 OR data into the line; the store retires from
//! the ROB by `mark_committed`; commit/flush drains one line per cycle.
//!
//! This matches the per-line byte-mask forwarding pattern used by Apple
//! M1/M2/M3, Intel Sunny Cove → Granite Rapids, AMD Zen 3/4/5, ARM Neoverse,
//! and BOOM. The structure is parallel to (not unified with) the scalar
//! `StoreBuffer`: vector stores never consume scalar SB slots.
//!
//! ## Forwarding semantics (`VecStoreForwarding::ByteMask`)
//!
//! - Load `[paddr, paddr+width)` against entries older than the load:
//!   - The youngest such entry touching the load decides: it holds every
//!     byte (`valid_mask & load_byte_mask == load_byte_mask`) → `Hit(data)`,
//!     some of them → `Stall`.
//!   - No entry touches it → `Miss`.
//! - Loads that straddle a 64-byte cache line never forward; they `Miss`.
//!
//! Memory-ordering violations against vec stores that have not yet resolved
//! all their elements are caught by the existing `LoadQueue` CAM at memory2,
//! not by the VSB. The VSB is intentionally optimistic on unresolved lines.
//!
//! ## Drain order
//!
//! `take_drainable_line` hands commit one cache-line buffer per cycle from
//! the oldest committed-and-fully-resolved entry, which commit writes as
//! one masked line write (or, for a device, as naturally aligned writes).
//! Between lines, the order is insertion order — which is element-index
//! order under the pipeline's FIFO memory path. This matches spike, ARM
//! SVE, and AVX-512.

use crate::common::PhysAddr;
use crate::core::pipeline::rob::RobTag;
use crate::core::pipeline::signals::MemWidth;
use crate::core::pipeline::store_buffer::{ForwardResult, width_to_bytes};
use crate::sim::components::ReqId;

/// Cache-line size used by the VSB. Matches the L1D line width.
pub const VSB_LINE_BYTES: usize = 64;

/// Forwarding policy. Selects how `forward_load` reacts to in-flight vec stores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VecStoreForwarding {
    /// Per-line byte-mask forwarding (BOOM/Apple/Intel/AMD/ARM pattern). Default.
    #[default]
    ByteMask,
    /// Saturn pattern: never forward; stall on overlap; miss otherwise.
    Stall,
    /// Most conservative: stall on any older in-flight vec store.
    Off,
}

/// What the vector store buffer can do for a vector load's span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanForward {
    /// One store holds all its bytes: the load takes them.
    Hit(Box<[u8]>),
    /// No store touches it: the load reads memory.
    Miss,
    /// A store holds some of its bytes: the load waits for its write.
    Stall,
}

/// One cache-line-aligned buffer inside a VSB entry.
///
/// `valid_mask` bit `i` set ⇔ `data[i]` was written by a resolved element
/// of the parent vec store. `line_addr` is `paddr & !(VSB_LINE_BYTES - 1)`.
#[derive(Clone, Debug)]
pub struct VsbLine {
    /// 64-byte-aligned base address of this line.
    pub line_addr: u64,
    /// Per-byte data; valid only at positions where `valid_mask` is set.
    pub data: [u8; VSB_LINE_BYTES],
    /// Bit `i` set ⇔ `data[i]` has been written by a resolved element.
    pub valid_mask: u64,
}

impl VsbLine {
    const fn new(line_addr: u64) -> Self {
        Self { line_addr, data: [0; VSB_LINE_BYTES], valid_mask: 0 }
    }

    /// The line's valid bytes as naturally aligned writes of 1, 2, 4 or 8
    /// bytes, `(address, data, width)`, for a device that must see each.
    #[must_use]
    pub fn natural_writes(&self) -> Vec<(PhysAddr, u64, MemWidth)> {
        let mut writes = Vec::new();
        let mut offset = 0usize;
        while offset < VSB_LINE_BYTES {
            if (self.valid_mask >> offset) & 1 == 0 {
                offset += 1;
                continue;
            }
            let run_end = (offset..VSB_LINE_BYTES)
                .find(|&i| (self.valid_mask >> i) & 1 == 0)
                .unwrap_or(VSB_LINE_BYTES);
            while offset < run_end {
                let addr = self.line_addr + offset as u64;
                let bytes = [8usize, 4, 2, 1]
                    .into_iter()
                    .find(|&n| {
                        addr.trailing_zeros() as usize >= n.trailing_zeros() as usize
                            && run_end - offset >= n
                    })
                    .unwrap_or(1);
                let data = (0..bytes)
                    .fold(0u64, |data, b| data | (u64::from(self.data[offset + b]) << (b * 8)));
                writes.push((PhysAddr::new(addr), data, mem_width_of(bytes)));
                offset += bytes;
            }
        }
        writes
    }
}

const fn mem_width_of(bytes: usize) -> MemWidth {
    match bytes {
        8 => MemWidth::Double,
        4 => MemWidth::Word,
        2 => MemWidth::Half,
        _ => MemWidth::Byte,
    }
}

/// One in-flight vector store instruction.
#[derive(Clone, Debug, Default)]
pub struct VecStoreBufferEntry {
    /// ROB tag of the parent vec store instruction.
    pub rob_tag: RobTag,
    /// Cache-line buffers; one per distinct 64-byte line touched.
    pub lines: Vec<VsbLine>,
    /// Active-element count, known once the store executes (may be less
    /// than `vl` if masked). `None` while the entry is only reserved.
    pub expected_elements: Option<usize>,
    /// Number of element-resolves received via `resolve_element`.
    pub resolved_elements: usize,
    /// `true` once the ROB has retired the parent vec store.
    pub committed: bool,
    /// Lines whose writes are sent and not all acknowledged: they still
    /// forward, since the memory system has not yet performed them.
    pub sent: Vec<SentVsbLine>,
    /// `true` while this slot occupies an in-flight entry.
    pub valid: bool,
}

/// A drained line and the writes carrying it that are not yet acknowledged.
#[derive(Clone, Debug)]
pub struct SentVsbLine {
    /// The line's bytes.
    pub line: VsbLine,
    /// Its writes still outstanding.
    pub writes: Vec<ReqId>,
}

impl VecStoreBufferEntry {
    /// The lines a younger load may forward from: those waiting to drain
    /// and those whose writes are in flight.
    fn forwarding_lines(&self) -> impl Iterator<Item = &VsbLine> {
        self.lines.iter().chain(self.sent.iter().map(|sent| &sent.line))
    }

    const fn is_committed_and_resolved(&self) -> bool {
        self.valid
            && self.committed
            && matches!(self.expected_elements, Some(expected) if expected == self.resolved_elements)
    }

    /// A committed store with lines still to write.
    const fn is_drainable(&self) -> bool {
        self.is_committed_and_resolved() && !self.lines.is_empty()
    }

    /// A committed store every write of which has been acknowledged.
    const fn is_finished(&self) -> bool {
        self.is_committed_and_resolved() && self.lines.is_empty() && self.sent.is_empty()
    }
}

/// Bounded buffer of in-flight vector stores. See module documentation.
#[derive(Debug)]
pub struct VecStoreBuffer {
    entries: Vec<VecStoreBufferEntry>,
    capacity: usize,
    forwarding: VecStoreForwarding,
}

impl VecStoreBuffer {
    /// Constructs a new buffer with the given capacity and forwarding policy.
    pub fn new(capacity: usize, forwarding: VecStoreForwarding) -> Self {
        Self { entries: Vec::with_capacity(capacity), capacity, forwarding }
    }

    /// Returns the configured maximum number of in-flight vector stores.
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the active forwarding policy.
    #[inline]
    pub const fn forwarding(&self) -> VecStoreForwarding {
        self.forwarding
    }

    /// Returns the number of in-flight entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| e.valid).count()
    }

    /// True while a committed store still has lines to write to memory.
    pub fn has_committed_stores(&self) -> bool {
        self.entries.iter().any(|e| e.valid && e.committed)
    }

    /// Returns true if there are no in-flight entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of free entry slots available for `allocate`.
    #[inline]
    pub fn free_slots(&self) -> usize {
        self.capacity.saturating_sub(self.len())
    }

    /// Reserves an entry for `rob_tag`. Called at rename, in program order,
    /// so a younger vector store can never hold the last slot an older one
    /// needs. Returns `false` if the buffer is at capacity, in which case
    /// rename must stall.
    ///
    /// Panics in debug builds if `rob_tag` is already present.
    pub fn allocate(&mut self, rob_tag: RobTag) -> bool {
        debug_assert!(
            !self.entries.iter().any(|e| e.valid && e.rob_tag == rob_tag),
            "VSB allocate: rob_tag {rob_tag:?} already present",
        );

        if self.len() >= self.capacity {
            return false;
        }

        // Reuse a slot whose entry was previously freed in-place, otherwise grow.
        let new_entry = VecStoreBufferEntry {
            rob_tag,
            lines: Vec::new(),
            expected_elements: None,
            resolved_elements: 0,
            committed: false,
            sent: Vec::new(),
            valid: true,
        };

        if let Some(slot) = self.entries.iter_mut().find(|e| !e.valid) {
            *slot = new_entry;
        } else {
            self.entries.push(new_entry);
        }

        true
    }

    /// Records how many element-resolves the store for `rob_tag` will
    /// deliver, once execute has evaluated `vl` and the mask. Zero (a fully
    /// masked store) makes the entry drainable as a no-op.
    pub fn set_expected_elements(&mut self, rob_tag: RobTag, expected_elements: usize) {
        let Some(entry) = self.entries.iter_mut().find(|e| e.valid && e.rob_tag == rob_tag) else {
            debug_assert!(false, "VSB set_expected_elements: no entry for {rob_tag:?}");
            return;
        };
        entry.expected_elements = Some(expected_elements);
    }

    /// Records one resolved element write. Splits across cache lines if
    /// `paddr + width` crosses a 64-byte boundary.
    ///
    /// Last writer wins per byte: later element writes to the same byte
    /// overwrite earlier ones. Spike walks elements in ascending index order
    /// and the memory pipeline is FIFO, so the natural call order matches
    /// spike — no per-byte sequence-number tracking is required.
    pub fn resolve_element(
        &mut self,
        rob_tag: RobTag,
        paddr: PhysAddr,
        data: u64,
        width: MemWidth,
    ) {
        let bytes = width_to_bytes(width);
        if bytes == 0 {
            return;
        }

        let Some(entry) = self.entries.iter_mut().find(|e| e.valid && e.rob_tag == rob_tag) else {
            debug_assert!(false, "VSB resolve_element: no entry for {rob_tag:?}");
            return;
        };

        let mut remaining = bytes;
        let mut cur_addr = paddr.val();
        let mut cur_data = data;

        while remaining > 0 {
            let line_addr = cur_addr & !(VSB_LINE_BYTES as u64 - 1);
            let offset = (cur_addr - line_addr) as usize;
            let take = remaining.min(VSB_LINE_BYTES - offset);

            let line_idx =
                entry.lines.iter().position(|l| l.line_addr == line_addr).unwrap_or_else(|| {
                    entry.lines.push(VsbLine::new(line_addr));
                    entry.lines.len() - 1
                });
            let line = &mut entry.lines[line_idx];

            for i in 0..take {
                let byte = ((cur_data >> (i * 8)) & 0xFF) as u8;
                line.data[offset + i] = byte;
                line.valid_mask |= 1u64 << (offset + i);
            }

            remaining -= take;
            cur_addr += take as u64;
            // Shifting a u64 by 64 is UB; an 8-byte element fitting in one line has no remainder.
            let shift_bits = take * 8;
            cur_data = if shift_bits >= 64 { 0 } else { cur_data >> shift_bits };
        }

        entry.resolved_elements += 1;
    }

    /// Marks the in-flight entry for `rob_tag` as committed (ROB has retired
    /// the parent vec store). The entry becomes drainable once all expected
    /// elements have also been resolved.
    pub fn mark_committed(&mut self, rob_tag: RobTag) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.valid && e.rob_tag == rob_tag) {
            entry.committed = true;
        }
        self.release_finished();
    }

    /// Returns `true` if the entry for `rob_tag` exists and has finished
    /// receiving all element-resolves. Used by callers that need to know
    /// whether a vec store can be safely drained at flush time.
    pub fn is_fully_resolved(&self, rob_tag: RobTag) -> bool {
        self.entries
            .iter()
            .find(|e| e.valid && e.rob_tag == rob_tag)
            .is_some_and(|e| e.expected_elements == Some(e.resolved_elements))
    }

    /// True when a vector store older than `rob_tag` has elements whose
    /// addresses are not yet known, so a load cannot tell whether it
    /// overlaps.
    pub fn has_unresolved_store_before(&self, rob_tag: RobTag) -> bool {
        self.entries.iter().any(|e| {
            e.valid
                && e.rob_tag.is_older_than(rob_tag)
                && e.expected_elements != Some(e.resolved_elements)
        })
    }

    /// True when a vector store older than `rob_tag` may write any of the
    /// `bytes` bytes at `paddr`: its addresses are not all known yet, or
    /// one of its lines, drained or not, holds such a byte.
    #[must_use]
    pub fn has_older_store_to(&self, paddr: PhysAddr, bytes: usize, rob_tag: RobTag) -> bool {
        let (start, end) = (paddr.val(), paddr.val() + bytes as u64);
        self.entries.iter().filter(|e| e.valid && e.rob_tag.is_older_than(rob_tag)).any(|e| {
            e.expected_elements != Some(e.resolved_elements)
                || e.forwarding_lines().any(|line| {
                    (0..VSB_LINE_BYTES as u64).any(|i| {
                        let byte = line.line_addr + i;
                        line.valid_mask >> i & 1 == 1 && byte >= start && byte < end
                    })
                })
        })
    }

    /// Forwarding check for a younger load. Policy-dependent — see module doc.
    pub fn forward_load(
        &self,
        paddr: PhysAddr,
        width: MemWidth,
        load_rob_tag: RobTag,
    ) -> ForwardResult {
        let bytes = width_to_bytes(width);
        if bytes == 0 {
            return ForwardResult::Miss;
        }

        let load_lo = paddr.val();
        let load_line = load_lo & !(VSB_LINE_BYTES as u64 - 1);
        let load_offset = (load_lo - load_line) as usize;

        // Cross-line loads never forward (matches real hardware penalties).
        if load_offset + bytes > VSB_LINE_BYTES {
            return self.cross_line_fallback(paddr, width, load_rob_tag);
        }

        let load_byte_mask: u64 =
            if bytes == VSB_LINE_BYTES { !0u64 } else { ((1u64 << bytes) - 1) << load_offset };

        match self.forwarding {
            VecStoreForwarding::ByteMask => self.forward_load_byte_mask(
                load_line,
                load_offset,
                bytes,
                load_byte_mask,
                load_rob_tag,
            ),
            VecStoreForwarding::Stall => {
                self.forward_load_stall(load_line, load_byte_mask, load_rob_tag)
            }
            VecStoreForwarding::Off => self.forward_load_off(load_rob_tag),
        }
    }

    /// Forwarding check for a vector load's span of `bytes` bytes at
    /// `paddr`, which lies in one line. Under `ByteMask` the youngest older
    /// store touching the span decides: it forwards when it holds every
    /// byte, and otherwise the load waits for it to be written.
    #[must_use]
    pub fn forward_span(&self, paddr: PhysAddr, bytes: usize, load_rob_tag: RobTag) -> SpanForward {
        let line_addr = paddr.val() & !(VSB_LINE_BYTES as u64 - 1);
        let offset = (paddr.val() - line_addr) as usize;
        let span_mask =
            if bytes >= VSB_LINE_BYTES { u64::MAX } else { ((1u64 << bytes) - 1) << offset };
        let older =
            self.entries.iter().filter(|e| e.valid && e.rob_tag.is_older_than(load_rob_tag));
        match self.forwarding {
            VecStoreForwarding::Off => {
                if older.count() > 0 {
                    SpanForward::Stall
                } else {
                    SpanForward::Miss
                }
            }
            VecStoreForwarding::Stall => {
                let touches = older
                    .flat_map(VecStoreBufferEntry::forwarding_lines)
                    .any(|line| line.line_addr == line_addr && line.valid_mask & span_mask != 0);
                if touches { SpanForward::Stall } else { SpanForward::Miss }
            }
            VecStoreForwarding::ByteMask => {
                let youngest = older
                    .filter_map(|e| {
                        let line = e.forwarding_lines().find(|l| l.line_addr == line_addr)?;
                        (line.valid_mask & span_mask != 0).then_some((e.rob_tag, line))
                    })
                    .reduce(|a, b| if b.0.is_newer_than(a.0) { b } else { a });
                match youngest {
                    None => SpanForward::Miss,
                    Some((_, line)) if line.valid_mask & span_mask == span_mask => {
                        SpanForward::Hit(line.data[offset..offset + bytes].into())
                    }
                    Some(_) => SpanForward::Stall,
                }
            }
        }
    }

    /// `Stall` if the load straddles a cache line and any older entry
    /// touches either line. Otherwise `Miss`. We don't attempt to merge
    /// cross-line forwards because real hardware doesn't either.
    fn cross_line_fallback(
        &self,
        paddr: PhysAddr,
        width: MemWidth,
        load_rob_tag: RobTag,
    ) -> ForwardResult {
        let bytes = width_to_bytes(width) as u64;
        let load_lo = paddr.val();
        let load_hi = load_lo + bytes;
        let line_a = load_lo & !(VSB_LINE_BYTES as u64 - 1);
        let line_b = (load_hi - 1) & !(VSB_LINE_BYTES as u64 - 1);

        for entry in self.entries.iter().filter(|e| e.valid) {
            if !entry.rob_tag.is_older_than(load_rob_tag) {
                continue;
            }
            for line in entry.forwarding_lines() {
                if (line.line_addr == line_a || line.line_addr == line_b) && line.valid_mask != 0 {
                    return ForwardResult::Stall;
                }
            }
            if self.forwarding == VecStoreForwarding::Off {
                return ForwardResult::Stall;
            }
        }
        ForwardResult::Miss
    }

    /// The youngest older store touching the load decides: it forwards
    /// when it holds every byte, and otherwise the load waits for it.
    fn forward_load_byte_mask(
        &self,
        load_line: u64,
        load_offset: usize,
        bytes: usize,
        load_byte_mask: u64,
        load_rob_tag: RobTag,
    ) -> ForwardResult {
        let youngest = self
            .entries
            .iter()
            .filter(|e| e.valid && e.rob_tag.is_older_than(load_rob_tag))
            .filter_map(|e| {
                let line = e.forwarding_lines().find(|l| l.line_addr == load_line)?;
                (line.valid_mask & load_byte_mask != 0).then_some((e.rob_tag, line))
            })
            .reduce(|a, b| if b.0.is_newer_than(a.0) { b } else { a });
        match youngest {
            None => ForwardResult::Miss,
            Some((_, line)) if line.valid_mask & load_byte_mask == load_byte_mask => {
                let data = (0..bytes).fold(0u64, |data, i| {
                    data | (u64::from(line.data[load_offset + i]) << (i * 8))
                });
                ForwardResult::Hit(data)
            }
            Some(_) => ForwardResult::Stall,
        }
    }

    fn forward_load_stall(
        &self,
        load_line: u64,
        load_byte_mask: u64,
        load_rob_tag: RobTag,
    ) -> ForwardResult {
        for entry in self.entries.iter().filter(|e| e.valid) {
            if !entry.rob_tag.is_older_than(load_rob_tag) {
                continue;
            }
            let unresolved_older =
                entry.expected_elements.is_none_or(|expected| entry.resolved_elements < expected);
            for line in entry.forwarding_lines() {
                if line.line_addr == load_line && (line.valid_mask & load_byte_mask) != 0 {
                    return ForwardResult::Stall;
                }
                if unresolved_older && line.line_addr == load_line {
                    return ForwardResult::Stall;
                }
            }
        }
        ForwardResult::Miss
    }

    fn forward_load_off(&self, load_rob_tag: RobTag) -> ForwardResult {
        for entry in self.entries.iter().filter(|e| e.valid) {
            if entry.rob_tag.is_older_than(load_rob_tag) {
                return ForwardResult::Stall;
            }
        }
        ForwardResult::Miss
    }

    /// Takes the next line to write: the first line of the oldest committed
    /// store whose elements have all resolved. The store keeps its slot
    /// until [`Self::line_sent`] records the line's writes and each of them
    /// is acknowledged.
    pub fn take_drainable_line(&mut self) -> Option<(RobTag, VsbLine)> {
        self.release_finished();
        let idx = self.oldest_drainable_entry_index()?;
        let entry = &mut self.entries[idx];
        let line = entry.lines.remove(0);
        entry.sent.push(SentVsbLine { line: line.clone(), writes: Vec::new() });
        Some((entry.rob_tag, line))
    }

    /// Records the writes carrying the line [`Self::take_drainable_line`]
    /// last handed out for `rob_tag`; it forwards until they are all
    /// acknowledged.
    pub fn line_sent(&mut self, rob_tag: RobTag, requests: Vec<ReqId>) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.valid && e.rob_tag == rob_tag)
            && let Some(sent) = entry.sent.last_mut()
        {
            sent.writes = requests;
            entry.sent.retain(|sent| !sent.writes.is_empty());
        }
        self.release_finished();
    }

    /// The memory system acknowledged `req`, one of a drained line's writes.
    pub fn write_acked(&mut self, req: ReqId) {
        for entry in &mut self.entries {
            for sent in &mut entry.sent {
                sent.writes.retain(|pending| *pending != req);
            }
            entry.sent.retain(|sent| !sent.writes.is_empty());
        }
        self.release_finished();
    }

    /// Frees the entries whose writes have all been acknowledged.
    fn release_finished(&mut self) {
        for entry in &mut self.entries {
            if entry.is_finished() {
                entry.valid = false;
            }
        }
    }

    /// Drops entries strictly newer than `keep_tag`. Older entries (whether
    /// committed or not) survive. Used on partial flush — branch
    /// misprediction or memory-ordering violation.
    pub fn flush_after(&mut self, keep_tag: RobTag) {
        for entry in &mut self.entries {
            if entry.valid && entry.rob_tag.is_newer_than(keep_tag) {
                entry.valid = false;
                entry.lines.clear();
            }
        }
    }

    /// Drops every entry that has not been committed. Committed entries stay
    /// and will continue draining on subsequent cycles. Called as part of a
    /// full speculative teardown.
    pub fn flush_speculative(&mut self) {
        for entry in &mut self.entries {
            if entry.valid && !entry.committed {
                entry.valid = false;
                entry.lines.clear();
            }
        }
    }

    /// Drops every entry, committed or not. Intended only for callers that
    /// have already drained committed work to memory.
    pub fn flush_all(&mut self) {
        for entry in &mut self.entries {
            entry.valid = false;
            entry.lines.clear();
            entry.sent.clear();
        }
    }

    fn oldest_drainable_entry_index(&self) -> Option<usize> {
        let mut chosen: Option<usize> = None;
        for (i, e) in self.entries.iter().enumerate() {
            if !e.is_drainable() {
                continue;
            }
            match chosen {
                None => chosen = Some(i),
                Some(prev) if e.rob_tag.is_older_than(self.entries[prev].rob_tag) => {
                    chosen = Some(i);
                }
                _ => {}
            }
        }
        chosen
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::PhysAddr;

    impl VecStoreBuffer {
        fn reserve_for_test(&mut self, rob_tag: RobTag, expected_elements: usize) -> bool {
            if !self.allocate(rob_tag) {
                return false;
            }
            self.set_expected_elements(rob_tag, expected_elements);
            true
        }
    }

    fn vsb(cap: usize) -> VecStoreBuffer {
        VecStoreBuffer::new(cap, VecStoreForwarding::ByteMask)
    }

    #[test]
    fn allocate_and_free_slots() {
        let mut b = vsb(2);
        assert_eq!(b.free_slots(), 2);
        assert!(b.reserve_for_test(RobTag(1), 4));
        assert_eq!(b.free_slots(), 1);
        assert!(b.reserve_for_test(RobTag(2), 4));
        assert_eq!(b.free_slots(), 0);
        assert!(!b.reserve_for_test(RobTag(3), 4));
    }

    #[test]
    fn resolve_single_line_word() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0xDEAD_BEEF, MemWidth::Word);

        // Forward a Word-aligned read from the same address — full hit.
        let result = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Word, RobTag(2));
        assert_eq!(result, ForwardResult::Hit(0xDEAD_BEEF));
        // A byte read from offset 0 returns the low byte.
        let result = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag(2));
        assert_eq!(result, ForwardResult::Hit(0xEF));
        // A byte read from offset 3 returns the high byte.
        let result = b.forward_load(PhysAddr::new(0x8000_0003), MemWidth::Byte, RobTag(2));
        assert_eq!(result, ForwardResult::Hit(0xDE));
    }

    #[test]
    fn resolve_cross_line_double() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 1);
        // Write 8 bytes starting 4 before a line boundary — splits across lines.
        b.resolve_element(
            RobTag(1),
            PhysAddr::new(0x8000_003C),
            0x0807_0605_0403_0201,
            MemWidth::Double,
        );

        // The first half is in line 0x8000_0000.
        let r = b.forward_load(PhysAddr::new(0x8000_003C), MemWidth::Word, RobTag(2));
        assert_eq!(r, ForwardResult::Hit(0x0403_0201));
        // The second half is in line 0x8000_0040.
        let r = b.forward_load(PhysAddr::new(0x8000_0040), MemWidth::Word, RobTag(2));
        assert_eq!(r, ForwardResult::Hit(0x0807_0605));
    }

    #[test]
    fn forward_partial_overlap_stalls() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0004), 0xAABB, MemWidth::Half);
        // Load Word at offset 0x8000_0002 overlaps bytes 4..6 of the line but
        // wants 4 bytes (2..6). Bytes 2..4 are not valid → partial overlap.
        let r = b.forward_load(PhysAddr::new(0x8000_0002), MemWidth::Word, RobTag(2));
        assert_eq!(r, ForwardResult::Stall);
    }

    #[test]
    fn forward_no_overlap_misses() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0xFF, MemWidth::Byte);
        let r = b.forward_load(PhysAddr::new(0x8000_0008), MemWidth::Word, RobTag(2));
        assert_eq!(r, ForwardResult::Miss);
    }

    #[test]
    fn youngest_older_match_wins() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 1);
        b.reserve_for_test(RobTag(2), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0x1111, MemWidth::Half);
        b.resolve_element(RobTag(2), PhysAddr::new(0x8000_0000), 0x2222, MemWidth::Half);

        let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Half, RobTag(3));
        assert_eq!(r, ForwardResult::Hit(0x2222));
    }

    #[test]
    fn a_younger_store_over_part_of_the_load_stalls_it() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 1);
        b.reserve_for_test(RobTag(2), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0x1111_1111, MemWidth::Word);
        b.resolve_element(RobTag(2), PhysAddr::new(0x8000_0002), 0x22, MemWidth::Byte);

        let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Word, RobTag(3));

        assert_eq!(r, ForwardResult::Stall, "store 2 overwrote a byte store 1 would forward");
    }

    #[test]
    fn newer_store_does_not_forward_to_older_load() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(5), 1);
        b.resolve_element(RobTag(5), PhysAddr::new(0x8000_0000), 0xABCD, MemWidth::Half);
        // Load tag 3 is older than store tag 5 — must not forward.
        let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Half, RobTag(3));
        assert_eq!(r, ForwardResult::Miss);
    }

    #[test]
    fn cross_line_load_does_not_forward() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 1);
        b.resolve_element(
            RobTag(1),
            PhysAddr::new(0x8000_003C),
            0x0102_0304_0506_0708,
            MemWidth::Double,
        );
        // Cross-line Word load (3 bytes in line A, 1 byte in line B): never forward.
        let r = b.forward_load(PhysAddr::new(0x8000_003D), MemWidth::Word, RobTag(2));
        assert_eq!(r, ForwardResult::Stall);
    }

    #[test]
    fn last_writer_wins_per_byte() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 2);
        // Two elements writing the same byte; the second call wins.
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0xBB, MemWidth::Byte);
        let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag(2));
        assert_eq!(r, ForwardResult::Hit(0xBB));
    }

    #[test]
    fn stall_policy_stalls_on_overlap() {
        let mut b = VecStoreBuffer::new(2, VecStoreForwarding::Stall);
        b.reserve_for_test(RobTag(1), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
        let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag(2));
        assert_eq!(r, ForwardResult::Stall);
        let r = b.forward_load(PhysAddr::new(0x8000_0008), MemWidth::Byte, RobTag(2));
        assert_eq!(r, ForwardResult::Miss);
    }

    #[test]
    fn off_policy_stalls_on_any_older_entry() {
        let mut b = VecStoreBuffer::new(2, VecStoreForwarding::Off);
        b.reserve_for_test(RobTag(1), 1);
        // Even before any element resolves, an older entry causes a stall.
        let r = b.forward_load(PhysAddr::new(0x9000_0000), MemWidth::Byte, RobTag(2));
        assert_eq!(r, ForwardResult::Stall);
    }

    #[test]
    fn mark_committed_does_not_change_forwarding() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
        b.mark_committed(RobTag(1));
        let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag(2));
        assert_eq!(r, ForwardResult::Hit(0xAA));
    }

    #[test]
    fn flush_after_drops_newer_entries() {
        let mut b = vsb(4);
        b.reserve_for_test(RobTag(1), 1);
        b.reserve_for_test(RobTag(2), 1);
        b.reserve_for_test(RobTag(3), 1);
        b.flush_after(RobTag(1));
        assert!(b.entries.iter().any(|e| e.valid && e.rob_tag == RobTag(1)));
        assert!(!b.entries.iter().any(|e| e.valid && e.rob_tag == RobTag(2)));
        assert!(!b.entries.iter().any(|e| e.valid && e.rob_tag == RobTag(3)));
    }

    #[test]
    fn flush_speculative_keeps_committed() {
        let mut b = vsb(4);
        b.reserve_for_test(RobTag(1), 1);
        b.reserve_for_test(RobTag(2), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
        b.mark_committed(RobTag(1));
        b.flush_speculative();
        assert!(b.entries.iter().any(|e| e.valid && e.rob_tag == RobTag(1)));
        assert!(!b.entries.iter().any(|e| e.valid && e.rob_tag == RobTag(2)));
    }

    #[test]
    fn a_drained_entry_keeps_its_slot_until_its_writes_are_acknowledged() {
        let mut b = vsb(4);
        b.reserve_for_test(RobTag(1), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
        b.mark_committed(RobTag(1));
        let entry = b.entries.iter_mut().find(|e| e.valid).expect("entry");
        let line = entry.lines.remove(0);
        entry.sent.push(SentVsbLine { line, writes: vec![ReqId::new(9)] });

        b.release_finished();
        let before_ack = (b.len(), b.has_committed_stores());
        b.write_acked(ReqId::new(9));

        assert_eq!((before_ack, b.len()), ((1, true), 0));
    }

    fn line_with(line_addr: u64, bytes: std::ops::Range<usize>) -> VsbLine {
        let mut line = VsbLine::new(line_addr);
        for i in bytes {
            line.data[i] = i as u8;
            line.valid_mask |= 1 << i;
        }
        line
    }

    #[test]
    fn a_device_write_splits_a_run_into_naturally_aligned_pieces() {
        let line = line_with(0x1000_0000, 3..16);

        let writes: Vec<_> =
            line.natural_writes().into_iter().map(|(a, d, w)| (a.val(), d, w)).collect();

        assert_eq!(
            writes,
            vec![
                (0x1000_0003, 0x03, MemWidth::Byte),
                (0x1000_0004, 0x0706_0504, MemWidth::Word),
                (0x1000_0008, 0x0F0E_0D0C_0B0A_0908, MemWidth::Double),
            ]
        );
    }

    #[test]
    fn a_device_write_skips_the_bytes_no_element_wrote() {
        let mut line = line_with(0x1000_0000, 0..2);
        line.valid_mask |= 1 << 4;

        let addresses: Vec<u64> = line.natural_writes().iter().map(|(a, _, _)| a.val()).collect();

        assert_eq!(addresses, vec![0x1000_0000, 0x1000_0004]);
    }

    #[test]
    fn a_drained_line_forwards_until_its_write_is_acknowledged() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0xAB, MemWidth::Byte);
        b.mark_committed(RobTag(1));
        let _ = b.take_drainable_line();
        b.line_sent(RobTag(1), vec![ReqId::new(7)]);

        let in_flight = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag(2));
        b.write_acked(ReqId::new(7));
        let written = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag(2));

        assert_eq!((in_flight, written), (ForwardResult::Hit(0xAB), ForwardResult::Miss));
    }

    #[test]
    fn an_older_vector_store_to_the_bytes_holds_an_lr_back() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 1);
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0004), 0xAB, MemWidth::Byte);

        let overlapping = b.has_older_store_to(PhysAddr::new(0x8000_0000), 8, RobTag(2));
        let elsewhere = b.has_older_store_to(PhysAddr::new(0x8000_0008), 8, RobTag(2));
        let younger = b.has_older_store_to(PhysAddr::new(0x8000_0000), 8, RobTag(1));

        assert_eq!((overlapping, elsewhere, younger), (true, false, false));
    }

    #[test]
    fn allocation_reuses_freed_slot() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 0);
        b.reserve_for_test(RobTag(2), 0);
        b.flush_speculative();
        assert_eq!(b.len(), 0);
        // Should reuse the freed slots, not grow.
        assert!(b.reserve_for_test(RobTag(3), 0));
        assert!(b.reserve_for_test(RobTag(4), 0));
        assert!(!b.reserve_for_test(RobTag(5), 0));
    }

    #[test]
    fn vec_store_forwarding_default_is_byte_mask() {
        let f = VecStoreForwarding::default();
        assert_eq!(f, VecStoreForwarding::ByteMask);
    }

    #[test]
    fn is_fully_resolved_tracks_progress() {
        let mut b = vsb(2);
        b.reserve_for_test(RobTag(1), 2);
        assert!(!b.is_fully_resolved(RobTag(1)));
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
        assert!(!b.is_fully_resolved(RobTag(1)));
        b.resolve_element(RobTag(1), PhysAddr::new(0x8000_0001), 0xBB, MemWidth::Byte);
        assert!(b.is_fully_resolved(RobTag(1)));
    }
}
