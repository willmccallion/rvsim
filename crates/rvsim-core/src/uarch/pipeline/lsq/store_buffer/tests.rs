//! Store buffer tests.

#![allow(clippy::unwrap_used, unused_results)]

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
