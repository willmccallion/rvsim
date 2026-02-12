//! Branch Target Buffer (BTB) Tests.
//!
//! Verifies lookup/update semantics, tag matching, aliasing behaviour,
//! and capacity-related edge cases for the direct-mapped BTB.

use riscv_core::core::units::bru::btb::Btb;

// ══════════════════════════════════════════════════════════
// 1. Basic lookup/update
// ══════════════════════════════════════════════════════════

#[test]
fn lookup_empty_returns_none() {
    let btb = Btb::new(16);
    assert_eq!(btb.lookup(0x1000), None);
}

#[test]
fn update_then_lookup() {
    let mut btb = Btb::new(16);
    btb.update(0x1000, 0x2000);
    assert_eq!(btb.lookup(0x1000), Some(0x2000));
}

#[test]
fn update_overwrites_previous_target() {
    let mut btb = Btb::new(16);
    btb.update(0x1000, 0x2000);
    btb.update(0x1000, 0x3000);
    assert_eq!(btb.lookup(0x1000), Some(0x3000), "Latest update should win");
}

// ══════════════════════════════════════════════════════════
// 2. Tag mismatch
// ══════════════════════════════════════════════════════════

#[test]
fn lookup_wrong_pc_returns_none() {
    let mut btb = Btb::new(16);
    btb.update(0x1000, 0x2000);
    assert_eq!(btb.lookup(0x1004), None, "Different PC should not match");
}

#[test]
fn lookup_after_aliasing_eviction() {
    // Two PCs that map to the same index (due to modular indexing)
    // but have different tags. The second update evicts the first.
    let mut btb = Btb::new(4); // 4 entries → index = (pc >> 2) & 3
    let pc_a = 0x1000; // index = (0x1000 >> 2) & 3 = 0
    let pc_b = 0x1010; // index = (0x1010 >> 2) & 3 = 0 (same index!)
    btb.update(pc_a, 0xAAAA);
    btb.update(pc_b, 0xBBBB);
    assert_eq!(btb.lookup(pc_a), None, "pc_a evicted by pc_b (same index)");
    assert_eq!(btb.lookup(pc_b), Some(0xBBBB));
}

// ══════════════════════════════════════════════════════════
// 3. Multiple distinct entries
// ══════════════════════════════════════════════════════════

#[test]
fn multiple_entries_non_conflicting() {
    let mut btb = Btb::new(64);
    btb.update(0x1000, 0xA);
    btb.update(0x1004, 0xB);
    btb.update(0x1008, 0xC);
    btb.update(0x100C, 0xD);
    assert_eq!(btb.lookup(0x1000), Some(0xA));
    assert_eq!(btb.lookup(0x1004), Some(0xB));
    assert_eq!(btb.lookup(0x1008), Some(0xC));
    assert_eq!(btb.lookup(0x100C), Some(0xD));
}

// ══════════════════════════════════════════════════════════
// 4. Size and indexing
// ══════════════════════════════════════════════════════════

#[test]
fn index_wraps_around() {
    // With size=8, index = (pc >> 2) & 7.
    // Verify that entries at different indices don't conflict.
    let mut btb = Btb::new(8);
    for i in 0u64..8 {
        let pc = i * 4; // Each goes to a unique index
        btb.update(pc, 0x1000 + i);
    }
    for i in 0u64..8 {
        let pc = i * 4;
        assert_eq!(
            btb.lookup(pc),
            Some(0x1000 + i),
            "Entry at index {i} should be intact"
        );
    }
}

#[test]
fn fill_entire_btb() {
    let size = 32;
    let mut btb = Btb::new(size);
    for i in 0..size as u64 {
        btb.update(i * 4, 0xF000 + i);
    }
    for i in 0..size as u64 {
        assert_eq!(btb.lookup(i * 4), Some(0xF000 + i));
    }
}

// ══════════════════════════════════════════════════════════
// 5. Edge cases
// ══════════════════════════════════════════════════════════

#[test]
fn lookup_pc_zero() {
    let mut btb = Btb::new(16);
    btb.update(0, 0x4000);
    assert_eq!(btb.lookup(0), Some(0x4000));
}

#[test]
fn lookup_high_address() {
    let mut btb = Btb::new(16);
    let high_pc = 0x8000_0000_0000_0000;
    btb.update(high_pc, 0xDEAD);
    assert_eq!(btb.lookup(high_pc), Some(0xDEAD));
}

#[test]
fn target_zero_is_valid() {
    let mut btb = Btb::new(16);
    btb.update(0x1000, 0);
    assert_eq!(btb.lookup(0x1000), Some(0), "Target address 0 is valid");
}

#[test]
fn target_max_is_valid() {
    let mut btb = Btb::new(16);
    btb.update(0x1000, u64::MAX);
    assert_eq!(btb.lookup(0x1000), Some(u64::MAX));
}

// ══════════════════════════════════════════════════════════
// 6. Realistic branch patterns
// ══════════════════════════════════════════════════════════

#[test]
fn loop_branch_updates_consistently() {
    // Simulate a loop: branch at 0x1008 always targets 0x1000.
    let mut btb = Btb::new(64);
    let branch_pc = 0x1008;
    let target = 0x1000;

    // First encounter — cold miss
    assert_eq!(btb.lookup(branch_pc), None);

    // After resolution, update the BTB
    btb.update(branch_pc, target);

    // Subsequent lookups hit
    for _ in 0..10 {
        assert_eq!(btb.lookup(branch_pc), Some(target));
    }
}

#[test]
fn switching_targets() {
    // An indirect jump might go to different targets.
    let mut btb = Btb::new(64);
    let pc = 0x2000;

    btb.update(pc, 0xA000);
    assert_eq!(btb.lookup(pc), Some(0xA000));

    btb.update(pc, 0xB000);
    assert_eq!(btb.lookup(pc), Some(0xB000));

    btb.update(pc, 0xC000);
    assert_eq!(btb.lookup(pc), Some(0xC000));
}
