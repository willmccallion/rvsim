//! A watched range of addresses scores a hit on an access of the kind it
//! watches that touches at least one of its bytes.

use crate::common::VirtAddr;
use crate::uarch::pipeline::latches::MemAccess;
use crate::uarch::{Watch, WatchHit, WatchKind};

const RANGE_START: u64 = 0x8000_1000;
const RANGE_BYTES: u64 = 16;
const DOUBLEWORD: u64 = 8;

const fn watch(kind: WatchKind) -> Watch {
    Watch { addr: RANGE_START, len: RANGE_BYTES, kind }
}

const fn read_of(vaddr: u64) -> MemAccess {
    MemAccess { vaddr: VirtAddr::new(vaddr), bytes: DOUBLEWORD, read: true, write: false }
}

#[test]
fn an_access_watch_hits_a_read() {
    let hit = watch(WatchKind::Access).hit(&read_of(RANGE_START));

    assert_eq!(hit, Some(WatchHit { addr: RANGE_START, kind: WatchKind::Access }));
}

#[test]
fn a_write_watch_ignores_a_read_of_its_range() {
    assert_eq!(watch(WatchKind::Write).hit(&read_of(RANGE_START)), None);
}

#[test]
fn a_read_ending_where_the_range_begins_misses_it() {
    assert_eq!(watch(WatchKind::Read).hit(&read_of(RANGE_START - DOUBLEWORD)), None);
}

#[test]
fn a_read_starting_where_the_range_ends_misses_it() {
    assert_eq!(watch(WatchKind::Read).hit(&read_of(RANGE_START + RANGE_BYTES)), None);
}
