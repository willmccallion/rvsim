//! Vector store buffer tests.

#![allow(clippy::unwrap_used, unused_results)]

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
    assert!(b.reserve_for_test(RobTag::new(1), 4));
    assert_eq!(b.free_slots(), 1);
    assert!(b.reserve_for_test(RobTag::new(2), 4));
    assert_eq!(b.free_slots(), 0);
    assert!(!b.reserve_for_test(RobTag::new(3), 4));
}

#[test]
fn resolve_single_line_word() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xDEAD_BEEF, MemWidth::Word);

    // Forward a Word-aligned read from the same address — full hit.
    let result = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Word, RobTag::new(2));
    assert_eq!(result, ForwardResult::Hit(0xDEAD_BEEF));
    // A byte read from offset 0 returns the low byte.
    let result = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag::new(2));
    assert_eq!(result, ForwardResult::Hit(0xEF));
    // A byte read from offset 3 returns the high byte.
    let result = b.forward_load(PhysAddr::new(0x8000_0003), MemWidth::Byte, RobTag::new(2));
    assert_eq!(result, ForwardResult::Hit(0xDE));
}

#[test]
fn resolve_cross_line_double() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    // Write 8 bytes starting 4 before a line boundary — splits across lines.
    b.resolve_element(
        RobTag::new(1),
        PhysAddr::new(0x8000_003C),
        0x0807_0605_0403_0201,
        MemWidth::Double,
    );

    // The first half is in line 0x8000_0000.
    let r = b.forward_load(PhysAddr::new(0x8000_003C), MemWidth::Word, RobTag::new(2));
    assert_eq!(r, ForwardResult::Hit(0x0403_0201));
    // The second half is in line 0x8000_0040.
    let r = b.forward_load(PhysAddr::new(0x8000_0040), MemWidth::Word, RobTag::new(2));
    assert_eq!(r, ForwardResult::Hit(0x0807_0605));
}

#[test]
fn forward_partial_overlap_stalls() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0004), 0xAABB, MemWidth::Half);
    // Load Word at offset 0x8000_0002 overlaps bytes 4..6 of the line but
    // wants 4 bytes (2..6). Bytes 2..4 are not valid → partial overlap.
    let r = b.forward_load(PhysAddr::new(0x8000_0002), MemWidth::Word, RobTag::new(2));
    assert_eq!(r, ForwardResult::Stall);
}

#[test]
fn forward_no_overlap_misses() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xFF, MemWidth::Byte);
    let r = b.forward_load(PhysAddr::new(0x8000_0008), MemWidth::Word, RobTag::new(2));
    assert_eq!(r, ForwardResult::Miss);
}

#[test]
fn youngest_older_match_wins() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    b.reserve_for_test(RobTag::new(2), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0x1111, MemWidth::Half);
    b.resolve_element(RobTag::new(2), PhysAddr::new(0x8000_0000), 0x2222, MemWidth::Half);

    let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Half, RobTag::new(3));
    assert_eq!(r, ForwardResult::Hit(0x2222));
}

#[test]
fn a_younger_store_over_part_of_the_load_stalls_it() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    b.reserve_for_test(RobTag::new(2), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0x1111_1111, MemWidth::Word);
    b.resolve_element(RobTag::new(2), PhysAddr::new(0x8000_0002), 0x22, MemWidth::Byte);

    let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Word, RobTag::new(3));

    assert_eq!(r, ForwardResult::Stall, "store 2 overwrote a byte store 1 would forward");
}

#[test]
fn newer_store_does_not_forward_to_older_load() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(5), 1);
    b.resolve_element(RobTag::new(5), PhysAddr::new(0x8000_0000), 0xABCD, MemWidth::Half);
    // Load tag 3 is older than store tag 5 — must not forward.
    let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Half, RobTag::new(3));
    assert_eq!(r, ForwardResult::Miss);
}

#[test]
fn cross_line_load_does_not_forward() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(
        RobTag::new(1),
        PhysAddr::new(0x8000_003C),
        0x0102_0304_0506_0708,
        MemWidth::Double,
    );
    // Cross-line Word load (3 bytes in line A, 1 byte in line B): never forward.
    let r = b.forward_load(PhysAddr::new(0x8000_003D), MemWidth::Word, RobTag::new(2));
    assert_eq!(r, ForwardResult::Stall);
}

#[test]
fn last_writer_wins_per_byte() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 2);
    // Two elements writing the same byte; the second call wins.
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xBB, MemWidth::Byte);
    let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag::new(2));
    assert_eq!(r, ForwardResult::Hit(0xBB));
}

#[test]
fn stall_policy_stalls_on_overlap() {
    let mut b = VecStoreBuffer::new(2, VecStoreForwarding::Stall);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
    let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag::new(2));
    assert_eq!(r, ForwardResult::Stall);
    let r = b.forward_load(PhysAddr::new(0x8000_0008), MemWidth::Byte, RobTag::new(2));
    assert_eq!(r, ForwardResult::Miss);
}

#[test]
fn off_policy_stalls_on_any_older_entry() {
    let mut b = VecStoreBuffer::new(2, VecStoreForwarding::Off);
    b.reserve_for_test(RobTag::new(1), 1);
    // Even before any element resolves, an older entry causes a stall.
    let r = b.forward_load(PhysAddr::new(0x9000_0000), MemWidth::Byte, RobTag::new(2));
    assert_eq!(r, ForwardResult::Stall);
}

#[test]
fn mark_committed_does_not_change_forwarding() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
    b.mark_committed(RobTag::new(1));
    let r = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag::new(2));
    assert_eq!(r, ForwardResult::Hit(0xAA));
}

#[test]
fn flush_after_drops_newer_entries() {
    let mut b = vsb(4);
    b.reserve_for_test(RobTag::new(1), 1);
    b.reserve_for_test(RobTag::new(2), 1);
    b.reserve_for_test(RobTag::new(3), 1);
    b.flush_after(RobTag::new(1));
    assert!(b.entries().any(|e| e.rob_tag == RobTag::new(1)));
    assert!(!b.entries().any(|e| e.rob_tag == RobTag::new(2)));
    assert!(!b.entries().any(|e| e.rob_tag == RobTag::new(3)));
}

#[test]
fn flush_speculative_keeps_committed() {
    let mut b = vsb(4);
    b.reserve_for_test(RobTag::new(1), 1);
    b.reserve_for_test(RobTag::new(2), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
    b.mark_committed(RobTag::new(1));
    b.flush_speculative();
    assert!(b.entries().any(|e| e.rob_tag == RobTag::new(1)));
    assert!(!b.entries().any(|e| e.rob_tag == RobTag::new(2)));
}

#[test]
fn a_drained_entry_keeps_its_slot_until_its_writes_are_acknowledged() {
    let mut b = vsb(4);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
    b.mark_committed(RobTag::new(1));
    let entry = b.slots.iter_mut().flatten().next().expect("entry");
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
        line.data[i] = u8::try_from(i).unwrap();
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
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xAB, MemWidth::Byte);
    b.mark_committed(RobTag::new(1));
    let _ = b.take_drainable_line();
    b.line_sent(RobTag::new(1), vec![ReqId::new(7)]);

    let in_flight = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag::new(2));
    b.write_acked(ReqId::new(7));
    let written = b.forward_load(PhysAddr::new(0x8000_0000), MemWidth::Byte, RobTag::new(2));

    assert_eq!((in_flight, written), (ForwardResult::Hit(0xAB), ForwardResult::Miss));
}

#[test]
fn an_older_vector_store_to_the_bytes_holds_an_lr_back() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0004), 0xAB, MemWidth::Byte);

    let overlapping = b.has_older_store_to(PhysAddr::new(0x8000_0000), 8, RobTag::new(2));
    let elsewhere = b.has_older_store_to(PhysAddr::new(0x8000_0008), 8, RobTag::new(2));
    let younger = b.has_older_store_to(PhysAddr::new(0x8000_0000), 8, RobTag::new(1));

    assert_eq!((overlapping, elsewhere, younger), (true, false, false));
}

#[test]
fn allocation_reuses_freed_slot() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 0);
    b.reserve_for_test(RobTag::new(2), 0);
    b.flush_speculative();
    assert_eq!(b.len(), 0);
    // Should reuse the freed slots, not grow.
    assert!(b.reserve_for_test(RobTag::new(3), 0));
    assert!(b.reserve_for_test(RobTag::new(4), 0));
    assert!(!b.reserve_for_test(RobTag::new(5), 0));
}

#[test]
fn vec_store_forwarding_default_is_byte_mask() {
    let f = VecStoreForwarding::default();
    assert_eq!(f, VecStoreForwarding::ByteMask);
}

#[test]
fn is_fully_resolved_tracks_progress() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 2);
    assert!(!b.is_fully_resolved(RobTag::new(1)));
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xAA, MemWidth::Byte);
    assert!(!b.is_fully_resolved(RobTag::new(1)));
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0001), 0xBB, MemWidth::Byte);
    assert!(b.is_fully_resolved(RobTag::new(1)));
}

#[test]
fn a_buffer_is_empty_until_a_store_is_allocated() {
    let mut b = vsb(2);
    assert!(b.is_empty());

    b.reserve_for_test(RobTag::new(1), 1);

    assert!(!b.is_empty());
}

#[test]
fn a_store_just_past_the_loaded_bytes_is_not_an_older_store_to_them() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0008), 0xAB, MemWidth::Byte);

    let before_the_store = b.has_older_store_to(PhysAddr::new(0x8000_0004), 4, RobTag::new(2));
    let up_to_the_store = b.has_older_store_to(PhysAddr::new(0x8000_0005), 4, RobTag::new(2));

    assert_eq!((before_the_store, up_to_the_store), (false, true));
}

#[test]
fn only_an_older_store_with_unresolved_elements_is_unresolved_before_a_load() {
    let mut b = vsb(4);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0000), 0xAB, MemWidth::Byte);
    b.reserve_for_test(RobTag::new(3), 1);

    let after_a_resolved_older_store = b.has_unresolved_store_before(RobTag::new(2));
    b.reserve_for_test(RobTag::new(0), 1);
    let after_an_unresolved_older_store = b.has_unresolved_store_before(RobTag::new(2));

    assert_eq!((after_a_resolved_older_store, after_an_unresolved_older_store), (false, true));
}

#[test]
fn an_older_line_still_to_send_holds_back_a_write_to_its_bytes_only() {
    let mut b = vsb(4);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0008), 0xAB, MemWidth::Byte);

    let over_the_byte = b.has_unsent_older_line_to(PhysAddr::new(0x8000_0008), 4, RobTag::new(2));
    let just_before = b.has_unsent_older_line_to(PhysAddr::new(0x8000_0000), 8, RobTag::new(2));
    let from_a_younger = b.has_unsent_older_line_to(PhysAddr::new(0x8000_0008), 4, RobTag::new(1));

    assert_eq!((over_the_byte, just_before, from_a_younger), (true, false, false));
}

#[test]
fn a_line_already_sent_no_longer_holds_back_a_write() {
    let mut b = vsb(4);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0008), 0xAB, MemWidth::Byte);
    b.mark_committed(RobTag::new(1));
    b.take_drainable_line().unwrap();

    assert!(!b.has_unsent_older_line_to(PhysAddr::new(0x8000_0008), 4, RobTag::new(2)));
}

#[test]
fn a_vector_store_between_a_scalar_store_and_a_load_is_the_loads_youngest_writer() {
    let mut b = vsb(4);
    b.reserve_for_test(RobTag::new(2), 1);
    b.resolve_element(RobTag::new(2), PhysAddr::new(0x8000_0008), 0xAB, MemWidth::Byte);
    let (scalar, load) = (RobTag::new(1), RobTag::new(3));

    let over_the_byte = b.writes_to_between(scalar, load, PhysAddr::new(0x8000_0008), 4);
    let elsewhere = b.writes_to_between(scalar, load, PhysAddr::new(0x8000_0000), 8);
    let older_than_the_scalar =
        b.writes_to_between(RobTag::new(2), load, PhysAddr::new(0x8000_0008), 4);
    let younger_than_the_load =
        b.writes_to_between(scalar, RobTag::new(2), PhysAddr::new(0x8000_0008), 4);

    assert_eq!(
        (over_the_byte, elsewhere, older_than_the_scalar, younger_than_the_load),
        (true, false, false, false)
    );
}

#[test]
fn an_element_inside_a_line_marks_only_its_own_bytes_of_that_line() {
    let mut b = vsb(2);
    b.reserve_for_test(RobTag::new(1), 1);
    b.resolve_element(RobTag::new(1), PhysAddr::new(0x8000_0025), 0xBEEF, MemWidth::Half);
    b.mark_committed(RobTag::new(1));

    let (_, line) = b.take_drainable_line().unwrap();

    assert_eq!(line.line_addr, 0x8000_0000);
    assert_eq!(line.valid_mask, 0b11 << 0x25);
}
