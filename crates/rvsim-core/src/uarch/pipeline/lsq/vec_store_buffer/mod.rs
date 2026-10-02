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
use crate::config::VecStoreForwarding;
use crate::isa::op::MemWidth;
use crate::sim::components::ReqId;
use crate::uarch::pipeline::lsq::store_buffer::{ForwardResult, width_to_bytes};
use crate::uarch::pipeline::rob::RobTag;

/// Cache-line size used by the VSB. Matches the L1D line width.
pub const VSB_LINE_BYTES: usize = 64;

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

    #[cfg(test)]
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
mod tests;
