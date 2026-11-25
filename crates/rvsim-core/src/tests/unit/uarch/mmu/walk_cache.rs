//! The walk cache on its own: the deepest table wins, the least recently
//! used entry goes when it is full, and an ASID flush spares global tables.

use crate::common::{Asid, VirtAddr};
use crate::uarch::mmu::walk_cache::{TableStart, WalkCache, WalkedPointer};

const VADDR: u64 = 0x4000_1234;
const ASID: Asid = Asid::new(7);
const OTHER_ASID: Asid = Asid::new(8);

fn pointer(vaddr: u64, level: u32, ppn: u64, is_global: bool) -> WalkedPointer {
    WalkedPointer {
        vaddr: VirtAddr::new(vaddr),
        asid: ASID,
        table: TableStart { level, ppn },
        is_global,
    }
}

#[test]
fn a_walk_starts_at_the_deepest_table_cached_for_its_address() {
    let mut cache = WalkCache::new(4);
    cache.insert(pointer(VADDR, 1, 0x100, false));
    cache.insert(pointer(VADDR, 0, 0x200, false));

    let start = cache.lookup(VirtAddr::new(VADDR + 0x1000), ASID);

    assert_eq!(start, Some(TableStart { level: 0, ppn: 0x200 }));
}

#[test]
fn a_table_serves_only_the_addresses_above_its_level_it_was_walked_for() {
    let mut cache = WalkCache::new(4);
    cache.insert(pointer(VADDR, 0, 0x200, false));

    let next_megapage = VirtAddr::new(VADDR + (2 << 20));

    assert_eq!(cache.lookup(next_megapage, ASID), None);
}

#[test]
fn a_table_serves_only_its_own_address_space_unless_global() {
    let mut cache = WalkCache::new(4);
    cache.insert(pointer(VADDR, 0, 0x200, false));
    cache.insert(pointer(VADDR, 1, 0x100, true));

    let start = cache.lookup(VirtAddr::new(VADDR), OTHER_ASID);

    assert_eq!(start, Some(TableStart { level: 1, ppn: 0x100 }));
}

#[test]
fn a_full_cache_replaces_its_least_recently_used_table() {
    let mut cache = WalkCache::new(2);
    let megapage = |n: u64| VADDR + (n << 21);
    cache.insert(pointer(megapage(0), 0, 0x200, false));
    cache.insert(pointer(megapage(1), 0, 0x201, false));
    let _ = cache.lookup(VirtAddr::new(megapage(0)), ASID);

    cache.insert(pointer(megapage(2), 0, 0x202, false));

    assert!(cache.lookup(VirtAddr::new(megapage(0)), ASID).is_some(), "used last");
    assert_eq!(cache.lookup(VirtAddr::new(megapage(1)), ASID), None, "least recently used");
    assert!(cache.lookup(VirtAddr::new(megapage(2)), ASID).is_some(), "just cached");
}

#[test]
fn walking_the_same_table_again_refreshes_its_entry_rather_than_adding_one() {
    let mut cache = WalkCache::new(2);
    cache.insert(pointer(VADDR, 0, 0x200, false));
    cache.insert(pointer(VADDR, 0, 0x300, false));
    cache.insert(pointer(VADDR, 1, 0x100, false));

    let start = cache.lookup(VirtAddr::new(VADDR), ASID);

    assert_eq!(start, Some(TableStart { level: 0, ppn: 0x300 }), "both entries still held");
}

#[test]
fn an_asid_flush_spares_global_tables_and_other_address_spaces() {
    let mut cache = WalkCache::new(4);
    let other = WalkedPointer { asid: OTHER_ASID, ..pointer(VADDR, 0, 0x200, false) };
    cache.insert(other);
    cache.insert(pointer(VADDR, 1, 0x100, true));
    cache.insert(pointer(VADDR, 0, 0x300, false));

    cache.flush_asid(ASID);

    assert_eq!(cache.lookup(VirtAddr::new(VADDR), ASID), Some(TableStart { level: 1, ppn: 0x100 }));
    assert_eq!(
        cache.lookup(VirtAddr::new(VADDR), OTHER_ASID),
        Some(TableStart { level: 0, ppn: 0x200 })
    );
}

#[test]
fn a_full_flush_empties_the_cache() {
    let mut cache = WalkCache::new(4);
    cache.insert(pointer(VADDR, 1, 0x100, true));

    cache.flush();

    assert_eq!(cache.lookup(VirtAddr::new(VADDR), ASID), None);
}

#[test]
fn a_machine_without_a_walk_cache_keeps_nothing() {
    let mut cache = WalkCache::new(0);
    cache.insert(pointer(VADDR, 1, 0x100, false));

    assert!(!cache.exists());
    assert_eq!(cache.lookup(VirtAddr::new(VADDR), ASID), None);
}
