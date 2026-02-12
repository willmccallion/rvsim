//! TLB Unit Tests.
//!
//! Verifies functionality of the Translation Lookaside Buffer:
//! - Basic lookup and insertion
//! - Permission bit extraction from PTE
//! - Aliasing eviction (same index)
//! - Capacity and full associativity (or lack thereof - TLB is direct mapped)
//! - Flushing

use riscv_core::core::units::mmu::tlb::Tlb;

// ══════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════

// PTE permission bits
const PTE_V: u64 = 1 << 0;
const PTE_R: u64 = 1 << 1;
const PTE_W: u64 = 1 << 2;
const PTE_X: u64 = 1 << 3;
const PTE_U: u64 = 1 << 4;

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
    assert_eq!(tlb.lookup(0x100), None);
}

#[test]
fn insert_and_lookup_hit() {
    let mut tlb = Tlb::new(16);
    let vpn = 0xABC;
    let ppn = 0x123;
    let pte = make_pte(true, false, true, false); // R=1, W=0, X=1, U=0

    tlb.insert(vpn, ppn, pte);

    match tlb.lookup(vpn) {
        Some((found_ppn, r, w, x, u)) => {
            assert_eq!(found_ppn, ppn);
            assert_eq!(r, true);
            assert_eq!(w, false);
            assert_eq!(x, true);
            assert_eq!(u, false);
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
    tlb.insert(0x10, 0x100, make_pte(true, false, false, false));
    let (_, r, w, x, u) = tlb.lookup(0x10).unwrap();
    assert_eq!((r, w, x, u), (true, false, false, false));

    // RW
    tlb.insert(0x11, 0x101, make_pte(true, true, false, false));
    let (_, r, w, x, u) = tlb.lookup(0x11).unwrap();
    assert_eq!((r, w, x, u), (true, true, false, false));

    // RX
    tlb.insert(0x12, 0x102, make_pte(true, false, true, false));
    let (_, r, w, x, u) = tlb.lookup(0x12).unwrap();
    assert_eq!((r, w, x, u), (true, false, true, false));

    // User bit
    tlb.insert(0x13, 0x103, make_pte(true, true, true, true));
    let (_, _, _, _, u) = tlb.lookup(0x13).unwrap();
    assert!(u);
}

// ══════════════════════════════════════════════════════════
// 3. Aliasing / Conflict Misses
// ══════════════════════════════════════════════════════════

#[test]
fn aliasing_eviction() {
    let size = 16;
    let mut tlb = Tlb::new(size);

    // VPN 0 and VPN 16 map to the same index (0 % 16 == 16 % 16 == 0)
    let vpn1 = 0;
    let vpn2 = size as u64;

    tlb.insert(vpn1, 0x100, PTE_V | PTE_R);
    assert!(tlb.lookup(vpn1).is_some());

    tlb.insert(vpn2, 0x200, PTE_V | PTE_R);
    assert!(tlb.lookup(vpn2).is_some());

    // vpn1 should have been evicted
    assert_eq!(
        tlb.lookup(vpn1),
        None,
        "Old entry should be evicted by alias"
    );
}

#[test]
fn tag_mismatch() {
    let size = 16;
    let mut tlb = Tlb::new(size);

    // Insert at index 0
    tlb.insert(0, 0x100, PTE_V | PTE_R);

    // Lookup different VPN that maps to index 0
    let alias_vpn = size as u64;
    assert_eq!(
        tlb.lookup(alias_vpn),
        None,
        "Tag mismatch should result in miss"
    );
}

// ══════════════════════════════════════════════════════════
// 4. Flushing
// ══════════════════════════════════════════════════════════

#[test]
fn flush_clears_entries() {
    let mut tlb = Tlb::new(16);
    tlb.insert(0x1, 0x100, PTE_V | PTE_R);
    tlb.insert(0x2, 0x200, PTE_V | PTE_R);

    assert!(tlb.lookup(0x1).is_some());
    assert!(tlb.lookup(0x2).is_some());

    tlb.flush();

    assert_eq!(tlb.lookup(0x1), None);
    assert_eq!(tlb.lookup(0x2), None);
}

// ══════════════════════════════════════════════════════════
// 5. Capacity
// ══════════════════════════════════════════════════════════

#[test]
fn fill_capacity() {
    let size = 32;
    let mut tlb = Tlb::new(size);

    for i in 0..size {
        tlb.insert(i as u64, 0x1000 + i as u64, PTE_V | PTE_R);
    }

    for i in 0..size {
        assert!(
            tlb.lookup(i as u64).is_some(),
            "Entry {} should be present",
            i
        );
    }
}
