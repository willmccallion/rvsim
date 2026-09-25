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
//!   - `valid_mask & load_byte_mask == load_byte_mask` (full coverage in one
//!     line of one entry) → `Hit(data)`. Youngest such match wins.
//!   - Some bytes covered but not all → `Stall`.
//!   - No bytes covered → `Miss`.
//! - Loads that straddle a 64-byte cache line never forward; they `Miss`.
//!
//! Memory-ordering violations against vec stores that have not yet resolved
//! all their elements are caught by the existing `LoadQueue` CAM at memory2,
//! not by the VSB. The VSB is intentionally optimistic on unresolved lines.
//!
//! ## Drain order
//!
//! `drain_one_committed` writes one cache-line buffer per cycle from the
//! oldest committed-and-fully-resolved entry. Within a line, contiguous
//! valid-byte runs are issued as 1/2/4/8-byte writes. Between lines, the
//! order is insertion order — which is element-index order under the
//! pipeline's FIFO memory path. This matches spike, ARM SVE, and AVX-512.

use crate::common::PhysAddr;
use crate::sim::CoreCtx;
use crate::core::pipeline::rob::RobTag;
use crate::core::pipeline::signals::MemWidth;
use crate::core::pipeline::store_buffer::{ForwardResult, width_to_bytes};

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
    /// `true` while this slot occupies an in-flight entry.
    pub valid: bool,
}

impl VecStoreBufferEntry {
    const fn is_drainable(&self) -> bool {
        self.valid
            && self.committed
            && matches!(self.expected_elements, Some(expected) if expected == self.resolved_elements)
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
            for line in &entry.lines {
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

    fn forward_load_byte_mask(
        &self,
        load_line: u64,
        load_offset: usize,
        bytes: usize,
        load_byte_mask: u64,
        load_rob_tag: RobTag,
    ) -> ForwardResult {
        let mut best_hit: Option<(RobTag, u64)> = None;
        let mut had_partial = false;

        for entry in self.entries.iter().filter(|e| e.valid) {
            if !entry.rob_tag.is_older_than(load_rob_tag) {
                continue;
            }
            for line in &entry.lines {
                if line.line_addr != load_line {
                    continue;
                }
                let coverage = line.valid_mask & load_byte_mask;
                if coverage == load_byte_mask {
                    let mut data: u64 = 0;
                    for i in 0..bytes {
                        data |= (line.data[load_offset + i] as u64) << (i * 8);
                    }
                    match best_hit {
                        None => best_hit = Some((entry.rob_tag, data)),
                        Some((prev_tag, _)) if entry.rob_tag.is_newer_than(prev_tag) => {
                            best_hit = Some((entry.rob_tag, data));
                        }
                        _ => {}
                    }
                } else if coverage != 0 {
                    had_partial = true;
                }
            }
        }

        if let Some((_, data)) = best_hit {
            return ForwardResult::Hit(data);
        }
        if had_partial {
            return ForwardResult::Stall;
        }
        ForwardResult::Miss
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
            for line in &entry.lines {
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

    /// Drains one cache-line buffer from the oldest drainable entry by
    /// emitting `MemReq` packets through the engine's `BackendCommon`.
    /// Returns `true` if a write was issued. One call per pipeline cycle
    /// to share commit-time bandwidth with the scalar SB.
    pub fn drain_one_committed(
        &mut self,
        state: &mut CoreCtx<'_>,
        common: &mut crate::core::pipeline::engine::BackendCommon,
    ) -> bool {
        let Some(idx) = self.oldest_drainable_entry_index() else { return false };

        let line = {
            let entry = &mut self.entries[idx];
            entry.lines.remove(0)
        };
        write_line_to_memory(state, common, &line);

        if self.entries[idx].lines.is_empty() {
            self.entries[idx].valid = false;
        }
        true
    }

    /// Drains all currently-drainable entries. Used by FENCE / SATP / FENCE.I
    /// (commit-time barriers) and by the trap-driven full flush.
    pub fn drain_all_committed(
        &mut self,
        state: &mut CoreCtx<'_>,
        common: &mut crate::core::pipeline::engine::BackendCommon,
    ) {
        while self.drain_one_committed(state, common) {}
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
    /// have already drained committed work to memory (`drain_all_committed`).
    pub fn flush_all(&mut self) {
        for entry in &mut self.entries {
            entry.valid = false;
            entry.lines.clear();
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

/// Issues memory writes for every contiguous valid-byte run in a VSB line.
///
/// Each run is rounded to a single 1/2/4/8-byte aligned `MemReq` that
/// covers it, matching the scalar SB drain path.
fn write_line_to_memory(
    state: &mut CoreCtx<'_>,
    common: &mut crate::core::pipeline::engine::BackendCommon,
    line: &VsbLine,
) {
    let mut i = 0usize;
    while i < VSB_LINE_BYTES {
        if (line.valid_mask >> i) & 1 == 0 {
            i += 1;
            continue;
        }
        let mut run = 1usize;
        while i + run < VSB_LINE_BYTES && (line.valid_mask >> (i + run)) & 1 == 1 {
            run += 1;
        }

        // Issue the run as a sequence of natural-aligned writes.
        let mut pos = 0usize;
        while pos < run {
            let abs_offset = i + pos;
            let abs_addr = line.line_addr + abs_offset as u64;
            let max_natural = if abs_addr.trailing_zeros() >= 3 && run - pos >= 8 {
                8
            } else if abs_addr.trailing_zeros() >= 2 && run - pos >= 4 {
                4
            } else if abs_addr.trailing_zeros() >= 1 && run - pos >= 2 {
                2
            } else {
                1
            };

            let width = match max_natural {
                8 => MemWidth::Double,
                4 => MemWidth::Word,
                2 => MemWidth::Half,
                _ => MemWidth::Byte,
            };
            let mut data: u64 = 0;
            for b in 0..max_natural {
                data |= (line.data[abs_offset + b] as u64) << (b * 8);
            }
            let paddr = PhysAddr::new(abs_addr);
            issue_drained_write(state, common, paddr, data, width);

            pos += max_natural;
        }
        i += run;
    }
}

/// Publishes a single VSB-drained write and emits its `MemReq` (op = Write).
/// MMIO addresses fall through the packet path only (no RAM-backed write).
fn issue_drained_write(
    state: &mut CoreCtx<'_>,
    common: &mut crate::core::pipeline::engine::BackendCommon,
    paddr: PhysAddr,
    data: u64,
    width: MemWidth,
) {
    use crate::core::pipeline::outstanding::OutstandingStore;
    use crate::sim::components::ComponentId;
    use crate::sim::packet::{AccessSize, MemOp, Packet, WriteData};

    let access_size = match width {
        MemWidth::Byte => AccessSize::B1,
        MemWidth::Half => AccessSize::B2,
        MemWidth::Word => AccessSize::B4,
        MemWidth::Double => AccessSize::B8,
        MemWidth::Nop => return,
    };

    state.publish_write(paddr, data, width);

    let req_id = common.alloc_req_id();
    let l1_d_id = common.l1_d_id;
    let pipeline_id = common.pipeline_id;
    let _ = common.outstanding_stores.insert(
        req_id,
        OutstandingStore { rob_tag: RobTag::default(), paddr },
    );
    let cycle = state.cycle;
    state.event_queue.schedule(
        cycle,
        ComponentId::Cache(l1_d_id),
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq {
            req_id,
            paddr,
            vaddr: None,
            size: access_size,
            op: MemOp::Write { data: WriteData::Small(data) },
        },
    );
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
