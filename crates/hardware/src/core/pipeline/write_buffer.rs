//! Write-combining buffer (WCB): a merging write buffer between the store
//! buffer and the L1D.
//!
//! A committed store leaving the store buffer merges into the entry for its
//! line instead of writing the L1D itself, so sequential stores (memcpy,
//! struct initialisation) reach the cache as one line write. An entry holds
//! its bytes until it is sent, and the hart's own loads read them from it:
//!
//! - when a store for another line needs its slot (the LRU entry goes);
//! - once every byte of its line is written;
//! - when a load needs bytes it holds only some of;
//! - when the store buffers leave the L1D write port idle, which is also
//!   how it empties before anything that waits for older stores.
//!
//! A sent line keeps forwarding until the L1D acknowledges that it has
//! written it, as a store-buffer entry does: the cache may still serve a
//! load from its old copy of the line while it fetches write permission.
//! Barriers wait for the acknowledgements too.

use crate::common::PhysAddr;
use crate::core::pipeline::store_buffer::ForwardResult;
use crate::sim::components::ReqId;

/// Largest span one entry covers: its byte mask is a `u64`.
const MAX_ENTRY_BYTES: usize = 64;

/// The bytes one entry has gathered for a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WcbLine {
    /// Physical address of the first byte the entry covers.
    pub line_addr: u64,
    /// The entry's bytes; those not in `mask` are meaningless.
    pub data: [u8; MAX_ENTRY_BYTES],
    /// Which bytes have been written, bit `i` for byte `i`.
    pub mask: u64,
}

impl WcbLine {
    const fn empty(line_addr: u64) -> Self {
        Self { line_addr, data: [0; MAX_ENTRY_BYTES], mask: 0 }
    }
}

/// A slot gathering stores for one line.
#[derive(Clone, Debug)]
struct WcbEntry {
    line: WcbLine,
    /// Access sequence of the last merge (larger is more recent).
    last_use: u64,
    /// A load needs this line written before it can read memory.
    send_requested: bool,
}

/// Write-combining buffer with a configurable number of entries.
#[derive(Debug)]
pub struct WriteCombiningBuffer {
    slots: Vec<Option<WcbEntry>>,
    entry_bytes: usize,
    next_use: u64,
    /// Lines sent to the L1D and not yet acknowledged, oldest first.
    in_flight: Vec<SentLine>,
}

/// A line sent to the L1D, whose write it has not yet acknowledged.
#[derive(Clone, Debug)]
struct SentLine {
    req: ReqId,
    line: WcbLine,
}

impl WriteCombiningBuffer {
    /// A WCB of `capacity` entries, each covering one `line_bytes` line (at
    /// most 64 bytes of it). A capacity of 0 disables the WCB and stores
    /// write the L1D themselves.
    pub fn new(capacity: usize, line_bytes: usize) -> Self {
        let entry_bytes = if line_bytes == 0 { 64 } else { line_bytes.min(MAX_ENTRY_BYTES) };
        Self { slots: vec![None; capacity], entry_bytes, next_use: 0, in_flight: Vec::new() }
    }

    /// Returns true if the WCB is disabled (0 entries).
    #[inline]
    pub const fn is_disabled(&self) -> bool {
        self.slots.is_empty()
    }

    /// Bytes one entry covers.
    #[must_use]
    pub const fn entry_bytes(&self) -> usize {
        self.entry_bytes
    }

    const fn entry_base(&self, addr: u64) -> u64 {
        addr & !(self.entry_bytes as u64 - 1)
    }

    const fn full_mask(&self) -> u64 {
        if self.entry_bytes >= 64 { u64::MAX } else { (1 << self.entry_bytes) - 1 }
    }

    /// Merges `bytes` bytes of `data` at `paddr`, which must lie in one
    /// entry's span of an enabled buffer. Returns the line whose slot the
    /// store took, which the caller must send.
    #[must_use]
    pub fn merge_store(&mut self, paddr: PhysAddr, data: u64, bytes: usize) -> Option<WcbLine> {
        debug_assert!(!self.is_disabled(), "merge into a disabled WCB");
        let base = self.entry_base(paddr.val());
        debug_assert!(
            paddr.val() - base + bytes as u64 <= self.entry_bytes as u64,
            "store spans two WCB entries"
        );
        let offset = (paddr.val() - base) as usize;
        self.next_use += 1;
        let now = self.next_use;
        let index = self
            .slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|e| e.line.line_addr == base))
            .or_else(|| self.slots.iter().position(Option::is_none));
        let (index, evicted) = if let Some(index) = index {
            (index, None)
        } else {
            let lru = self.least_recently_used()?;
            (lru, self.slots[lru].take().map(|entry| entry.line))
        };
        let entry = self.slots[index].get_or_insert_with(|| WcbEntry {
            line: WcbLine::empty(base),
            last_use: now,
            send_requested: false,
        });
        entry.last_use = now;
        for (i, byte) in data.to_le_bytes().iter().take(bytes).enumerate() {
            entry.line.data[offset + i] = *byte;
            entry.line.mask |= 1 << (offset + i);
        }
        evicted
    }

    fn least_recently_used(&self) -> Option<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_ref().map(|entry| (i, entry.last_use)))
            .min_by_key(|&(_, last_use)| last_use)
            .map(|(i, _)| i)
    }

    /// Forwards to a load of `bytes` bytes at `paddr` the bytes the buffer
    /// holds, the newest store's for each byte. A load it covers only partly
    /// waits, and the lines it is waiting for are marked to be sent.
    pub fn forward_load(&mut self, paddr: PhysAddr, bytes: usize) -> ForwardResult {
        let mut value = 0u64;
        let mut found = 0;
        for i in 0..bytes {
            if let Some(byte) = self.newest_byte(paddr.val() + i as u64) {
                value |= u64::from(byte) << (8 * i);
                found += 1;
            }
        }
        if found == 0 {
            return ForwardResult::Miss;
        }
        if found == bytes {
            return ForwardResult::Hit(value);
        }
        let _ = self.request_send(paddr, bytes);
        ForwardResult::Stall
    }

    /// The byte at `addr` from the newest store the buffer holds for it: a
    /// line still merging, else the most recently sent line.
    fn newest_byte(&self, addr: u64) -> Option<u8> {
        let in_line = |line: &WcbLine| {
            let offset = addr.checked_sub(line.line_addr)?;
            (offset < self.entry_bytes as u64 && line.mask >> offset & 1 == 1)
                .then(|| line.data[offset as usize])
        };
        self.slots
            .iter()
            .flatten()
            .find_map(|entry| in_line(&entry.line))
            .or_else(|| self.in_flight.iter().rev().find_map(|sent| in_line(&sent.line)))
    }

    /// Marks every merging line holding any of the `bytes` bytes at `paddr`
    /// to be sent, for an access that must follow them to the cache.
    /// Returns whether any held or unacknowledged store covers one of them.
    pub fn request_send(&mut self, paddr: PhysAddr, bytes: usize) -> bool {
        let start = paddr.val();
        let end = start + bytes as u64;
        let span = self.entry_bytes as u64;
        let covers = |line: &WcbLine| {
            (0..span).filter(|&i| line.mask >> i & 1 == 1).any(|i| {
                let byte = line.line_addr + i;
                byte >= start && byte < end
            })
        };
        let mut overlapped = self.in_flight.iter().any(|sent| covers(&sent.line));
        for entry in self.slots.iter_mut().flatten() {
            if covers(&entry.line) {
                entry.send_requested = true;
                overlapped = true;
            }
        }
        overlapped
    }

    /// Takes a line that must go now: one a load is waiting for, or one
    /// that is fully written.
    pub fn take_urgent(&mut self) -> Option<WcbLine> {
        let full = self.full_mask();
        let index = self.slots.iter().position(|slot| {
            slot.as_ref().is_some_and(|entry| entry.send_requested || entry.line.mask == full)
        })?;
        self.slots[index].take().map(|entry| entry.line)
    }

    /// Takes the least recently merged line, to send while the write port
    /// is idle.
    pub fn take_oldest(&mut self) -> Option<WcbLine> {
        let index = self.least_recently_used()?;
        self.slots[index].take().map(|entry| entry.line)
    }

    /// Records that `req` carries `line`, which keeps forwarding until the
    /// L1D has written it.
    pub fn sent(&mut self, req: ReqId, line: WcbLine) {
        self.in_flight.push(SentLine { req, line });
    }

    /// The L1D acknowledged `req`: its line has been written.
    pub fn acked(&mut self, req: ReqId) {
        self.in_flight.retain(|sent| sent.req != req);
    }

    /// True while a store is held or a sent line is unacknowledged.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.in_flight.is_empty() || self.slots.iter().any(Option::is_some)
    }

    /// Lines holding stores.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.slots.iter().flatten().count()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;

    fn wcb(capacity: usize) -> WriteCombiningBuffer {
        WriteCombiningBuffer::new(capacity, 64)
    }

    #[test]
    fn stores_to_one_line_share_an_entry() {
        let mut wcb = wcb(4);
        assert!(wcb.merge_store(PhysAddr::new(0x1000), 0xAA, 1).is_none());
        assert!(wcb.merge_store(PhysAddr::new(0x1008), 0xBB, 1).is_none());
        assert_eq!(wcb.active_count(), 1);
    }

    #[test]
    fn a_store_to_a_new_line_evicts_the_least_recently_merged_one() {
        let mut wcb = wcb(2);
        let _ = wcb.merge_store(PhysAddr::new(0x1000), 1, 4);
        let _ = wcb.merge_store(PhysAddr::new(0x1040), 2, 4);
        let _ = wcb.merge_store(PhysAddr::new(0x1004), 3, 4);

        let evicted = wcb.merge_store(PhysAddr::new(0x1080), 4, 4).unwrap();

        assert_eq!(evicted.line_addr, 0x1040);
        assert_eq!(evicted.mask, 0xF);
        assert_eq!(wcb.active_count(), 2);
    }

    #[test]
    fn a_load_it_covers_reads_the_merged_bytes() {
        let mut wcb = wcb(2);
        let _ = wcb.merge_store(PhysAddr::new(0x1000), 0x1122, 2);
        let _ = wcb.merge_store(PhysAddr::new(0x1002), 0x3344, 2);

        assert_eq!(wcb.forward_load(PhysAddr::new(0x1000), 4), ForwardResult::Hit(0x3344_1122));
        assert_eq!(wcb.forward_load(PhysAddr::new(0x1004), 4), ForwardResult::Miss);
    }

    #[test]
    fn a_load_it_covers_partly_waits_and_gets_the_line_sent() {
        let mut wcb = wcb(2);
        let _ = wcb.merge_store(PhysAddr::new(0x1000), 0x11, 1);

        assert_eq!(wcb.forward_load(PhysAddr::new(0x1000), 2), ForwardResult::Stall);
        let sent = wcb.take_urgent().unwrap();

        assert_eq!(sent.line_addr, 0x1000);
        assert_eq!(wcb.forward_load(PhysAddr::new(0x1000), 1), ForwardResult::Miss);
    }

    #[test]
    fn a_fully_written_line_is_sent_at_once() {
        let mut wcb = wcb(2);
        for offset in (0..64).step_by(8) {
            let _ = wcb.merge_store(PhysAddr::new(0x2000 + offset), offset, 8);
        }

        assert_eq!(wcb.take_urgent().map(|line| line.mask), Some(u64::MAX));
    }

    #[test]
    fn a_sent_line_forwards_and_is_pending_until_acknowledged() {
        let mut wcb = wcb(2);
        let _ = wcb.merge_store(PhysAddr::new(0x1000), 1, 8);
        let line = wcb.take_oldest().unwrap();
        wcb.sent(ReqId::new(7), line);
        assert!(wcb.has_pending());
        assert_eq!(wcb.forward_load(PhysAddr::new(0x1000), 8), ForwardResult::Hit(1));

        wcb.acked(ReqId::new(7));

        assert!(!wcb.has_pending());
        assert_eq!(wcb.forward_load(PhysAddr::new(0x1000), 8), ForwardResult::Miss);
    }
}
