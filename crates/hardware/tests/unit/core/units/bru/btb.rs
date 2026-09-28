//! Branch Target Buffer (BTB) Tests.
//!
//! Verifies lookup/update semantics, tag matching, aliasing behaviour,
//! and capacity-related edge cases for both direct-mapped and
//! set-associative BTB configurations.

use rvsim_core::core::units::bru::btb::{BranchKind, Btb};

#[test]
fn lookup_empty_returns_none() {
    let btb = Btb::new(16, 1);
    assert_eq!(btb.lookup(0x1000).map(|hit| hit.target), None);
}

#[test]
fn update_then_lookup() {
    let mut btb = Btb::new(16, 1);
    btb.update(0x1000, 0x2000, BranchKind::Conditional);
    assert_eq!(btb.lookup(0x1000).map(|hit| hit.target), Some(0x2000));
}

#[test]
fn update_overwrites_previous_target() {
    let mut btb = Btb::new(16, 1);
    btb.update(0x1000, 0x2000, BranchKind::Conditional);
    btb.update(0x1000, 0x3000, BranchKind::Conditional);
    assert_eq!(btb.lookup(0x1000).map(|hit| hit.target), Some(0x3000), "Latest update should win");
}

#[test]
fn lookup_wrong_pc_returns_none() {
    let mut btb = Btb::new(16, 1);
    btb.update(0x1000, 0x2000, BranchKind::Conditional);
    assert_eq!(btb.lookup(0x1004).map(|hit| hit.target), None, "Different PC should not match");
}

#[test]
fn lookup_after_aliasing_eviction_direct_mapped() {
    // Direct-mapped: two PCs at same index but different tags → second evicts first.
    let mut btb = Btb::new(4, 1); // 4 sets, 1 way
    let pc_a = 0x1000; // index = (0x1000 >> 2) & 3 = 0
    let pc_b = 0x1010; // index = (0x1010 >> 2) & 3 = 0 (same index!)
    btb.update(pc_a, 0xAAAA, BranchKind::Conditional);
    btb.update(pc_b, 0xBBBB, BranchKind::Conditional);
    assert_eq!(btb.lookup(pc_a).map(|hit| hit.target), None, "pc_a evicted by pc_b (same index)");
    assert_eq!(btb.lookup(pc_b).map(|hit| hit.target), Some(0xBBBB));
}

#[test]
fn multiple_entries_non_conflicting() {
    let mut btb = Btb::new(64, 1);
    btb.update(0x1000, 0xA, BranchKind::Conditional);
    btb.update(0x1004, 0xB, BranchKind::Conditional);
    btb.update(0x1008, 0xC, BranchKind::Conditional);
    btb.update(0x100C, 0xD, BranchKind::Conditional);
    assert_eq!(btb.lookup(0x1000).map(|hit| hit.target), Some(0xA));
    assert_eq!(btb.lookup(0x1004).map(|hit| hit.target), Some(0xB));
    assert_eq!(btb.lookup(0x1008).map(|hit| hit.target), Some(0xC));
    assert_eq!(btb.lookup(0x100C).map(|hit| hit.target), Some(0xD));
}

#[test]
fn index_wraps_around() {
    let mut btb = Btb::new(8, 1);
    for i in 0u64..8 {
        let pc = i * 4;
        btb.update(pc, 0x1000 + i, BranchKind::Conditional);
    }
    for i in 0u64..8 {
        let pc = i * 4;
        assert_eq!(
            btb.lookup(pc).map(|hit| hit.target),
            Some(0x1000 + i),
            "Entry at index {i} should be intact"
        );
    }
}

#[test]
fn fill_entire_btb() {
    let size = 32;
    let mut btb = Btb::new(size, 1);
    for i in 0..size as u64 {
        btb.update(i * 4, 0xF000 + i, BranchKind::Conditional);
    }
    for i in 0..size as u64 {
        assert_eq!(btb.lookup(i * 4).map(|hit| hit.target), Some(0xF000 + i));
    }
}

#[test]
fn lookup_pc_zero() {
    let mut btb = Btb::new(16, 1);
    btb.update(0, 0x4000, BranchKind::Conditional);
    assert_eq!(btb.lookup(0).map(|hit| hit.target), Some(0x4000));
}

#[test]
fn lookup_high_address() {
    let mut btb = Btb::new(16, 1);
    let high_pc = 0x8000_0000_0000_0000;
    btb.update(high_pc, 0xDEAD, BranchKind::Conditional);
    assert_eq!(btb.lookup(high_pc).map(|hit| hit.target), Some(0xDEAD));
}

#[test]
fn target_zero_is_valid() {
    let mut btb = Btb::new(16, 1);
    btb.update(0x1000, 0, BranchKind::Conditional);
    assert_eq!(btb.lookup(0x1000).map(|hit| hit.target), Some(0), "Target address 0 is valid");
}

#[test]
fn target_max_is_valid() {
    let mut btb = Btb::new(16, 1);
    btb.update(0x1000, u64::MAX, BranchKind::Conditional);
    assert_eq!(btb.lookup(0x1000).map(|hit| hit.target), Some(u64::MAX));
}

#[test]
fn loop_branch_updates_consistently() {
    let mut btb = Btb::new(64, 1);
    let branch_pc = 0x1008;
    let target = 0x1000;

    assert_eq!(btb.lookup(branch_pc).map(|hit| hit.target), None);
    btb.update(branch_pc, target, BranchKind::Conditional);
    for _ in 0..10 {
        assert_eq!(btb.lookup(branch_pc).map(|hit| hit.target), Some(target));
    }
}

#[test]
fn switching_targets() {
    let mut btb = Btb::new(64, 1);
    let pc = 0x2000;

    btb.update(pc, 0xA000, BranchKind::Conditional);
    assert_eq!(btb.lookup(pc).map(|hit| hit.target), Some(0xA000));

    btb.update(pc, 0xB000, BranchKind::Conditional);
    assert_eq!(btb.lookup(pc).map(|hit| hit.target), Some(0xB000));

    btb.update(pc, 0xC000, BranchKind::Conditional);
    assert_eq!(btb.lookup(pc).map(|hit| hit.target), Some(0xC000));
}

#[test]
fn set_associative_no_conflict_within_ways() {
    // 4-way: two PCs in the same set should both fit.
    let mut btb = Btb::new(16, 4); // 4 sets, 4 ways
    let pc_a = 0x1000; // set = (0x1000 >> 2) & 3 = 0
    let pc_b = 0x1010; // set = (0x1010 >> 2) & 3 = 0 (same set!)
    btb.update(pc_a, 0xAAAA, BranchKind::Conditional);
    btb.update(pc_b, 0xBBBB, BranchKind::Conditional);
    assert_eq!(
        btb.lookup(pc_a).map(|hit| hit.target),
        Some(0xAAAA),
        "Both should coexist in 4-way set"
    );
    assert_eq!(btb.lookup(pc_b).map(|hit| hit.target), Some(0xBBBB));
}

#[test]
fn set_associative_evicts_when_full() {
    // 2-way: a third PC in the same set evicts the LRU entry.
    let mut btb = Btb::new(4, 2); // 2 sets, 2 ways
    // All three PCs need to map to the same set (set = (pc >> 2) & 1).
    let pc_a = 0x0000; // set = 0
    let pc_b = 0x0008; // set = 0
    let pc_c = 0x0010; // set = 0
    btb.update(pc_a, 0xA, BranchKind::Conditional);
    btb.update(pc_b, 0xB, BranchKind::Conditional);
    // Both should be present.
    assert_eq!(btb.lookup(pc_a).map(|hit| hit.target), Some(0xA));
    assert_eq!(btb.lookup(pc_b).map(|hit| hit.target), Some(0xB));
    // Third evicts one (round-robin: should evict pc_a at way 0).
    btb.update(pc_c, 0xC, BranchKind::Conditional);
    assert_eq!(btb.lookup(pc_a).map(|hit| hit.target), None, "pc_a should be evicted");
    assert_eq!(btb.lookup(pc_b).map(|hit| hit.target), Some(0xB));
    assert_eq!(btb.lookup(pc_c).map(|hit| hit.target), Some(0xC));
}

#[test]
fn set_associative_update_in_place() {
    // Updating an existing entry in a set should modify target, not allocate.
    let mut btb = Btb::new(16, 4);
    let pc = 0x1000;
    btb.update(pc, 0xAAAA, BranchKind::Conditional);
    btb.update(pc, 0xBBBB, BranchKind::Conditional);
    assert_eq!(btb.lookup(pc).map(|hit| hit.target), Some(0xBBBB));
}

#[test]
fn set_associative_different_sets_independent() {
    let mut btb = Btb::new(16, 4); // 4 sets, 4 ways
    let pc_set0 = 0x1000; // set 0
    let pc_set1 = 0x1004; // set 1
    btb.update(pc_set0, 0xA, BranchKind::Conditional);
    btb.update(pc_set1, 0xB, BranchKind::Conditional);
    assert_eq!(btb.lookup(pc_set0).map(|hit| hit.target), Some(0xA));
    assert_eq!(btb.lookup(pc_set1).map(|hit| hit.target), Some(0xB));
}
