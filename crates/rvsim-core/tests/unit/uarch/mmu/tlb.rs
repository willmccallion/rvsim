//! TLB Unit Tests.
//!
//! Verifies functionality of the Translation Lookaside Buffer:
//! - Basic lookup and insertion
//! - Permission bit extraction from PTE
//! - Full associativity with LRU replacement
//! - Superpage entries
//! - Flushing
//! - ASID tagging and global bit behavior

use rvsim_core::common::{Asid, Ppn, Vpn};
use rvsim_core::uarch::mmu::tlb::{PageSize, Tlb, TlbGeometry, TlbHit};

// PTE permission bits
const PTE_V: u64 = 1 << 0;
const PTE_R: u64 = 1 << 1;
const PTE_W: u64 = 1 << 2;
const PTE_X: u64 = 1 << 3;
const PTE_U: u64 = 1 << 4;
const PTE_G: u64 = 1 << 5;

fn fully_associative(entries: usize) -> Tlb {
    Tlb::new(TlbGeometry { entries, ways: 0 })
}

/// Helper to create a PTE with specific permissions
fn make_pte(r: bool, w: bool, x: bool, u: bool) -> u64 {
    let mut pte = PTE_V;
    if r {
        pte |= PTE_R;
    }
    if w {
        pte |= PTE_W;
    }
    if x {
        pte |= PTE_X;
    }
    if u {
        pte |= PTE_U;
    }
    pte
}

#[test]
fn lookup_miss_on_empty() {
    let mut tlb = fully_associative(16);
    assert_eq!(tlb.lookup(Vpn::new(0x100), Asid::new(0)), None);
}

#[test]
fn insert_and_lookup_hit() {
    let mut tlb = fully_associative(16);
    let vpn = Vpn::new(0xABC);
    let ppn = Ppn::new(0x123);
    let pte = make_pte(true, false, true, false); // R=1, W=0, X=1, U=0

    tlb.insert(vpn, ppn, pte, Asid::new(0), PageSize::Kib4);

    match tlb.lookup(vpn, Asid::new(0)) {
        Some(TlbHit { ppn: found_ppn, r, w, x, u, .. }) => {
            assert_eq!(found_ppn, ppn);
            assert!(r);
            assert!(!w);
            assert!(x);
            assert!(!u);
        }
        None => panic!("Should hit after insert"),
    }
}

#[test]
fn permissions_extracted_correctly() {
    let mut tlb = fully_associative(16);

    // R-only
    tlb.insert(
        Vpn::new(0x10),
        Ppn::new(0x100),
        make_pte(true, false, false, false),
        Asid::new(0),
        PageSize::Kib4,
    );
    let hit = tlb.lookup(Vpn::new(0x10), Asid::new(0)).unwrap();
    assert_eq!((hit.r, hit.w, hit.x, hit.u), (true, false, false, false));

    // RW
    tlb.insert(
        Vpn::new(0x11),
        Ppn::new(0x101),
        make_pte(true, true, false, false),
        Asid::new(0),
        PageSize::Kib4,
    );
    let hit = tlb.lookup(Vpn::new(0x11), Asid::new(0)).unwrap();
    assert_eq!((hit.r, hit.w, hit.x, hit.u), (true, true, false, false));

    // RX
    tlb.insert(
        Vpn::new(0x12),
        Ppn::new(0x102),
        make_pte(true, false, true, false),
        Asid::new(0),
        PageSize::Kib4,
    );
    let hit = tlb.lookup(Vpn::new(0x12), Asid::new(0)).unwrap();
    assert_eq!((hit.r, hit.w, hit.x, hit.u), (true, false, true, false));

    // User bit
    tlb.insert(
        Vpn::new(0x13),
        Ppn::new(0x103),
        make_pte(true, true, true, true),
        Asid::new(0),
        PageSize::Kib4,
    );
    assert!(tlb.lookup(Vpn::new(0x13), Asid::new(0)).unwrap().u);
}

#[test]
fn flush_clears_entries() {
    let mut tlb = fully_associative(16);
    tlb.insert(Vpn::new(0x1), Ppn::new(0x100), PTE_V | PTE_R, Asid::new(0), PageSize::Kib4);
    tlb.insert(Vpn::new(0x2), Ppn::new(0x200), PTE_V | PTE_R, Asid::new(0), PageSize::Kib4);

    assert!(tlb.lookup(Vpn::new(0x1), Asid::new(0)).is_some());
    assert!(tlb.lookup(Vpn::new(0x2), Asid::new(0)).is_some());

    tlb.flush();

    assert_eq!(tlb.lookup(Vpn::new(0x1), Asid::new(0)), None);
    assert_eq!(tlb.lookup(Vpn::new(0x2), Asid::new(0)), None);
}

#[test]
fn fill_capacity() {
    let size = 32;
    let mut tlb = fully_associative(size);

    for i in 0..size {
        tlb.insert(
            Vpn::new(i as u64),
            Ppn::new(0x1000 + i as u64),
            PTE_V | PTE_R,
            Asid::new(0),
            PageSize::Kib4,
        );
    }

    for i in 0..size {
        assert!(
            tlb.lookup(Vpn::new(i as u64), Asid::new(0)).is_some(),
            "Entry {} should be present",
            i
        );
    }
}

#[test]
fn asid_isolation() {
    let mut tlb = fully_associative(256);
    let vpn = Vpn::new(0x42);

    tlb.insert(vpn, Ppn::new(0x100), PTE_V | PTE_R, Asid::new(1), PageSize::Kib4);
    assert!(tlb.lookup(vpn, Asid::new(1)).is_some(), "Same ASID should hit");
    assert_eq!(tlb.lookup(vpn, Asid::new(2)), None, "Different ASID should miss");
}

#[test]
fn global_bit_matches_any_asid() {
    let mut tlb = fully_associative(256);
    let vpn = Vpn::new(0x42);

    // Insert with Global bit set
    tlb.insert(vpn, Ppn::new(0x100), PTE_V | PTE_R | PTE_G, Asid::new(1), PageSize::Kib4);
    assert!(tlb.lookup(vpn, Asid::new(1)).is_some(), "Same ASID should hit");
    assert!(tlb.lookup(vpn, Asid::new(2)).is_some(), "Different ASID should hit (global)");
    assert!(tlb.lookup(vpn, Asid::new(0)).is_some(), "ASID 0 should hit (global)");
}

#[test]
fn flush_asid_only_affects_matching() {
    let mut tlb = fully_associative(256);

    // Insert entries with different ASIDs at different VPNs
    tlb.insert(Vpn::new(0x10), Ppn::new(0x100), PTE_V | PTE_R, Asid::new(1), PageSize::Kib4);
    tlb.insert(Vpn::new(0x20), Ppn::new(0x200), PTE_V | PTE_R, Asid::new(2), PageSize::Kib4);

    tlb.flush_asid(Asid::new(1));

    assert_eq!(tlb.lookup(Vpn::new(0x10), Asid::new(1)), None, "ASID 1 entry should be flushed");
    assert!(tlb.lookup(Vpn::new(0x20), Asid::new(2)).is_some(), "ASID 2 entry should survive");
}

#[test]
fn flush_asid_preserves_global() {
    let mut tlb = fully_associative(256);

    tlb.insert(
        Vpn::new(0x10),
        Ppn::new(0x100),
        PTE_V | PTE_R | PTE_G,
        Asid::new(1),
        PageSize::Kib4,
    );
    tlb.flush_asid(Asid::new(1));

    assert!(
        tlb.lookup(Vpn::new(0x10), Asid::new(1)).is_some(),
        "Global entry should survive ASID flush"
    );
}

#[test]
fn flush_vaddr_asid() {
    let mut tlb = fully_associative(256);

    tlb.insert(Vpn::new(0x10), Ppn::new(0x100), PTE_V | PTE_R, Asid::new(1), PageSize::Kib4);
    tlb.insert(Vpn::new(0x20), Ppn::new(0x200), PTE_V | PTE_R, Asid::new(1), PageSize::Kib4);

    tlb.flush_vaddr_asid(Vpn::new(0x10), Asid::new(1));

    assert_eq!(tlb.lookup(Vpn::new(0x10), Asid::new(1)), None, "Targeted entry should be flushed");
    assert!(tlb.lookup(Vpn::new(0x20), Asid::new(1)).is_some(), "Other entry should survive");
}

#[test]
fn flush_vaddr_asid_preserves_global() {
    let mut tlb = fully_associative(256);

    tlb.insert(
        Vpn::new(0x10),
        Ppn::new(0x100),
        PTE_V | PTE_R | PTE_G,
        Asid::new(1),
        PageSize::Kib4,
    );
    tlb.flush_vaddr_asid(Vpn::new(0x10), Asid::new(1));

    assert!(
        tlb.lookup(Vpn::new(0x10), Asid::new(1)).is_some(),
        "Global entry should survive vaddr+ASID flush"
    );
}

#[test]
fn vpns_that_share_low_bits_coexist() {
    let mut tlb = fully_associative(16);

    tlb.insert(Vpn::new(0), Ppn::new(0x100), PTE_V | PTE_R, Asid::new(0), PageSize::Kib4);
    tlb.insert(Vpn::new(16), Ppn::new(0x200), PTE_V | PTE_R, Asid::new(0), PageSize::Kib4);

    let both = (tlb.lookup(Vpn::new(0), Asid::new(0)), tlb.lookup(Vpn::new(16), Asid::new(0)));
    assert!(both.0.is_some() && both.1.is_some(), "a fully associative TLB holds both");
}

#[test]
fn the_least_recently_used_entry_is_evicted_when_full() {
    let mut tlb = fully_associative(4);
    for vpn in 0..4 {
        tlb.insert(
            Vpn::new(vpn),
            Ppn::new(0x100 + vpn),
            PTE_V | PTE_R,
            Asid::new(0),
            PageSize::Kib4,
        );
    }
    let _ = tlb.lookup(Vpn::new(0), Asid::new(0));

    tlb.insert(Vpn::new(9), Ppn::new(0x900), PTE_V | PTE_R, Asid::new(0), PageSize::Kib4);

    assert!(tlb.lookup(Vpn::new(0), Asid::new(0)).is_some(), "recently used entry kept");
    assert_eq!(tlb.lookup(Vpn::new(1), Asid::new(0)), None, "least recently used evicted");
}

#[test]
fn a_superpage_entry_translates_every_page_it_covers() {
    let mut tlb = fully_associative(4);
    // A 2 MiB page: VPN 0x200..0x400 maps to PPN 0x8_0000..0x8_0200.
    tlb.insert(Vpn::new(0x234), Ppn::new(0x8_0034), PTE_V | PTE_R, Asid::new(0), PageSize::Mib2);

    let first = tlb.lookup(Vpn::new(0x200), Asid::new(0)).map(|hit| hit.ppn);
    let last = tlb.lookup(Vpn::new(0x3FF), Asid::new(0)).map(|hit| hit.ppn);
    let outside = tlb.lookup(Vpn::new(0x400), Asid::new(0));

    assert_eq!((first, last, outside), (Some(Ppn::new(0x8_0000)), Some(Ppn::new(0x8_01FF)), None));
}

#[test]
fn flushing_an_address_removes_the_superpage_covering_it() {
    let mut tlb = fully_associative(4);
    tlb.insert(Vpn::new(0x200), Ppn::new(0x8_0000), PTE_V | PTE_R, Asid::new(1), PageSize::Mib2);

    tlb.flush_vaddr(Vpn::new(0x3A0));

    assert_eq!(tlb.lookup(Vpn::new(0x200), Asid::new(1)), None);
}

#[test]
fn a_set_associative_tlb_holds_superpages_alongside_base_pages() {
    let mut l2 = Tlb::new(TlbGeometry { entries: 64, ways: 4 });
    l2.insert(Vpn::new(0x40000), Ppn::new(0x80000), PTE_V | PTE_R, Asid::new(0), PageSize::Gib1);
    l2.insert(Vpn::new(0x123), Ppn::new(0x456), PTE_V | PTE_R, Asid::new(0), PageSize::Kib4);

    let in_gigapage = l2.lookup(Vpn::new(0x4_1234), Asid::new(0)).map(|hit| hit.ppn);
    let base_page = l2.lookup(Vpn::new(0x123), Asid::new(0)).map(|hit| hit.ppn);

    assert_eq!((in_gigapage, base_page), (Some(Ppn::new(0x8_1234)), Some(Ppn::new(0x456))));
}

#[test]
fn a_tlb_with_no_entries_never_hits() {
    let mut tlb = Tlb::new(TlbGeometry { entries: 0, ways: 0 });

    tlb.insert(Vpn::new(1), Ppn::new(2), PTE_V | PTE_R, Asid::new(0), PageSize::Kib4);

    assert_eq!(tlb.lookup(Vpn::new(1), Asid::new(0)), None);
}

#[test]
fn a_direct_mapped_tlb_evicts_on_an_index_conflict() {
    let mut tlb = Tlb::new(TlbGeometry { entries: 16, ways: 1 });

    tlb.insert(Vpn::new(0), Ppn::new(0x100), PTE_V | PTE_R, Asid::new(0), PageSize::Kib4);
    tlb.insert(Vpn::new(16), Ppn::new(0x200), PTE_V | PTE_R, Asid::new(0), PageSize::Kib4);

    assert_eq!(tlb.lookup(Vpn::new(0), Asid::new(0)), None);
}
