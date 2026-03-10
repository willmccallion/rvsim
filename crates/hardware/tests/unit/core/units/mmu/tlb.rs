//! TLB Unit Tests.
//!
//! Verifies functionality of the Translation Lookaside Buffer:
//! - Basic lookup and insertion
//! - Permission bit extraction from PTE
//! - Aliasing eviction (same index)
//! - Capacity and full associativity (or lack thereof - TLB is direct mapped)
//! - Flushing
//! - ASID tagging and global bit behavior

use rvsim_core::common::{Asid, Ppn, Vpn};
use rvsim_core::core::units::mmu::tlb::{Tlb, TlbHit};

// ══════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════

// PTE permission bits
const PTE_V: u64 = 1 << 0;
const PTE_R: u64 = 1 << 1;
const PTE_W: u64 = 1 << 2;
const PTE_X: u64 = 1 << 3;
const PTE_U: u64 = 1 << 4;
const PTE_G: u64 = 1 << 5;

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

// ══════════════════════════════════════════════════════════
// 1. Basic Operations
// ══════════════════════════════════════════════════════════

#[test]
fn lookup_miss_on_empty() {
    let tlb = Tlb::new(16);
    assert_eq!(tlb.lookup(Vpn::new(0x100), Asid::new(0)), None);
}

#[test]
fn insert_and_lookup_hit() {
    let mut tlb = Tlb::new(16);
    let vpn = Vpn::new(0xABC);
    let ppn = Ppn::new(0x123);
    let pte = make_pte(true, false, true, false); // R=1, W=0, X=1, U=0

    tlb.insert(vpn, ppn, pte, Asid::new(0));

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

// ══════════════════════════════════════════════════════════
// 2. Permission Bit Extraction
// ══════════════════════════════════════════════════════════

#[test]
fn permissions_extracted_correctly() {
    let mut tlb = Tlb::new(16);

    // R-only
    tlb.insert(Vpn::new(0x10), Ppn::new(0x100), make_pte(true, false, false, false), Asid::new(0));
    let hit = tlb.lookup(Vpn::new(0x10), Asid::new(0)).unwrap();
    assert_eq!((hit.r, hit.w, hit.x, hit.u), (true, false, false, false));

    // RW
    tlb.insert(Vpn::new(0x11), Ppn::new(0x101), make_pte(true, true, false, false), Asid::new(0));
    let hit = tlb.lookup(Vpn::new(0x11), Asid::new(0)).unwrap();
    assert_eq!((hit.r, hit.w, hit.x, hit.u), (true, true, false, false));

    // RX
    tlb.insert(Vpn::new(0x12), Ppn::new(0x102), make_pte(true, false, true, false), Asid::new(0));
    let hit = tlb.lookup(Vpn::new(0x12), Asid::new(0)).unwrap();
    assert_eq!((hit.r, hit.w, hit.x, hit.u), (true, false, true, false));

    // User bit
    tlb.insert(Vpn::new(0x13), Ppn::new(0x103), make_pte(true, true, true, true), Asid::new(0));
    assert!(tlb.lookup(Vpn::new(0x13), Asid::new(0)).unwrap().u);
}

// ══════════════════════════════════════════════════════════
// 3. Aliasing / Conflict Misses
// ══════════════════════════════════════════════════════════

#[test]
fn aliasing_eviction() {
    let size = 16;
    let mut tlb = Tlb::new(size);

    // VPN 0 and VPN 16 map to the same index (0 % 16 == 16 % 16 == 0)
    let vpn1 = Vpn::new(0);
    let vpn2 = Vpn::new(size as u64);

    tlb.insert(vpn1, Ppn::new(0x100), PTE_V | PTE_R, Asid::new(0));
    assert!(tlb.lookup(vpn1, Asid::new(0)).is_some());

    tlb.insert(vpn2, Ppn::new(0x200), PTE_V | PTE_R, Asid::new(0));
    assert!(tlb.lookup(vpn2, Asid::new(0)).is_some());

    // vpn1 should have been evicted
    assert_eq!(tlb.lookup(vpn1, Asid::new(0)), None, "Old entry should be evicted by alias");
}

#[test]
fn tag_mismatch() {
    let size = 16;
    let mut tlb = Tlb::new(size);

    // Insert at index 0
    tlb.insert(Vpn::new(0), Ppn::new(0x100), PTE_V | PTE_R, Asid::new(0));

    // Lookup different VPN that maps to index 0
    let alias_vpn = Vpn::new(size as u64);
    assert_eq!(tlb.lookup(alias_vpn, Asid::new(0)), None, "Tag mismatch should result in miss");
}

// ══════════════════════════════════════════════════════════
// 4. Flushing
// ══════════════════════════════════════════════════════════

#[test]
fn flush_clears_entries() {
    let mut tlb = Tlb::new(16);
    tlb.insert(Vpn::new(0x1), Ppn::new(0x100), PTE_V | PTE_R, Asid::new(0));
    tlb.insert(Vpn::new(0x2), Ppn::new(0x200), PTE_V | PTE_R, Asid::new(0));

    assert!(tlb.lookup(Vpn::new(0x1), Asid::new(0)).is_some());
    assert!(tlb.lookup(Vpn::new(0x2), Asid::new(0)).is_some());

    tlb.flush();

    assert_eq!(tlb.lookup(Vpn::new(0x1), Asid::new(0)), None);
    assert_eq!(tlb.lookup(Vpn::new(0x2), Asid::new(0)), None);
}

// ══════════════════════════════════════════════════════════
// 5. Capacity
// ══════════════════════════════════════════════════════════

#[test]
fn fill_capacity() {
    let size = 32;
    let mut tlb = Tlb::new(size);

    for i in 0..size {
        tlb.insert(Vpn::new(i as u64), Ppn::new(0x1000 + i as u64), PTE_V | PTE_R, Asid::new(0));
    }

    for i in 0..size {
        assert!(
            tlb.lookup(Vpn::new(i as u64), Asid::new(0)).is_some(),
            "Entry {} should be present",
            i
        );
    }
}

// ══════════════════════════════════════════════════════════
// 6. ASID Tagging
// ══════════════════════════════════════════════════════════

#[test]
fn asid_isolation() {
    let mut tlb = Tlb::new(256);
    let vpn = Vpn::new(0x42);

    // Insert same VPN with different ASIDs into different index slots
    // Since TLB is direct-mapped, same VPN maps to same index — last write wins.
    // Instead, test that lookup with wrong ASID misses.
    tlb.insert(vpn, Ppn::new(0x100), PTE_V | PTE_R, Asid::new(1));
    assert!(tlb.lookup(vpn, Asid::new(1)).is_some(), "Same ASID should hit");
    assert_eq!(tlb.lookup(vpn, Asid::new(2)), None, "Different ASID should miss");
}

#[test]
fn global_bit_matches_any_asid() {
    let mut tlb = Tlb::new(256);
    let vpn = Vpn::new(0x42);

    // Insert with Global bit set
    tlb.insert(vpn, Ppn::new(0x100), PTE_V | PTE_R | PTE_G, Asid::new(1));
    assert!(tlb.lookup(vpn, Asid::new(1)).is_some(), "Same ASID should hit");
    assert!(tlb.lookup(vpn, Asid::new(2)).is_some(), "Different ASID should hit (global)");
    assert!(tlb.lookup(vpn, Asid::new(0)).is_some(), "ASID 0 should hit (global)");
}

#[test]
fn flush_asid_only_affects_matching() {
    let mut tlb = Tlb::new(256);

    // Insert entries with different ASIDs at different VPNs
    tlb.insert(Vpn::new(0x10), Ppn::new(0x100), PTE_V | PTE_R, Asid::new(1));
    tlb.insert(Vpn::new(0x20), Ppn::new(0x200), PTE_V | PTE_R, Asid::new(2));

    tlb.flush_asid(Asid::new(1));

    assert_eq!(tlb.lookup(Vpn::new(0x10), Asid::new(1)), None, "ASID 1 entry should be flushed");
    assert!(tlb.lookup(Vpn::new(0x20), Asid::new(2)).is_some(), "ASID 2 entry should survive");
}

#[test]
fn flush_asid_preserves_global() {
    let mut tlb = Tlb::new(256);

    tlb.insert(Vpn::new(0x10), Ppn::new(0x100), PTE_V | PTE_R | PTE_G, Asid::new(1));
    tlb.flush_asid(Asid::new(1));

    assert!(
        tlb.lookup(Vpn::new(0x10), Asid::new(1)).is_some(),
        "Global entry should survive ASID flush"
    );
}

#[test]
fn flush_vaddr_asid() {
    let mut tlb = Tlb::new(256);

    tlb.insert(Vpn::new(0x10), Ppn::new(0x100), PTE_V | PTE_R, Asid::new(1));
    tlb.insert(Vpn::new(0x20), Ppn::new(0x200), PTE_V | PTE_R, Asid::new(1));

    tlb.flush_vaddr_asid(Vpn::new(0x10), Asid::new(1));

    assert_eq!(tlb.lookup(Vpn::new(0x10), Asid::new(1)), None, "Targeted entry should be flushed");
    assert!(tlb.lookup(Vpn::new(0x20), Asid::new(1)).is_some(), "Other entry should survive");
}

#[test]
fn flush_vaddr_asid_preserves_global() {
    let mut tlb = Tlb::new(256);

    tlb.insert(Vpn::new(0x10), Ppn::new(0x100), PTE_V | PTE_R | PTE_G, Asid::new(1));
    tlb.flush_vaddr_asid(Vpn::new(0x10), Asid::new(1));

    assert!(
        tlb.lookup(Vpn::new(0x10), Asid::new(1)).is_some(),
        "Global entry should survive vaddr+ASID flush"
    );
}
