//! Cache Replacement Policy Tests.
//!
//! Verifies the victim selection logic for LRU, FIFO, PLRU, MRU, and Random policies.
//! Each policy implements `ReplacementPolicy` with `update(set, way)` and
//! `get_victim(set) -> usize`. Tests exercise them in isolation with edge cases.
//!
//! Reference: Phase 3 — Memory Subsystem Verification.

use riscv_core::core::units::cache::policies::{
    FifoPolicy, LruPolicy, MruPolicy, PlruPolicy, RandomPolicy, ReplacementPolicy,
};

// ══════════════════════════════════════════════════════════
// 1. LRU Policy
// ══════════════════════════════════════════════════════════

/// LRU initial state: ways are in order [0,1,2,3], so the last way (3)
/// is conceptually the "most recent" and way 3 is the last in stack.
/// Actually, in LruPolicy::new, stack is (0..ways).collect() = [0,1,2,3]
/// with index 0 = MRU. So get_victim returns *last() = 3.
#[test]
fn lru_initial_victim_is_last_way() {
    let mut policy = LruPolicy::new(1, 4);
    // No accesses yet; initial stack is [0, 1, 2, 3].
    // MRU = 0, LRU = 3.
    assert_eq!(policy.get_victim(0), 3);
}

/// Accessing ways in order 0,1,2,3 makes 0 the LRU.
#[test]
fn lru_sequential_access_reorders() {
    let mut policy = LruPolicy::new(1, 4);

    policy.update(0, 0);
    policy.update(0, 1);
    policy.update(0, 2);
    policy.update(0, 3);
    // Stack: [3, 2, 1, 0]. LRU = 0.
    assert_eq!(policy.get_victim(0), 0);
}

/// Classic LRU scenario: access 0,1,2,3 then re-access 0 → LRU becomes 1.
#[test]
fn lru_evicts_true_lru_after_reaccess() {
    let mut policy = LruPolicy::new(1, 4);

    policy.update(0, 0);
    policy.update(0, 1);
    policy.update(0, 2);
    policy.update(0, 3);
    // Stack: [3, 2, 1, 0]. LRU = 0.
    assert_eq!(policy.get_victim(0), 0);

    // Re-access 0 → promotes to MRU.
    policy.update(0, 0);
    // Stack: [0, 3, 2, 1]. LRU = 1.
    assert_eq!(policy.get_victim(0), 1);

    // Re-access 1 → promotes to MRU.
    policy.update(0, 1);
    // Stack: [1, 0, 3, 2]. LRU = 2.
    assert_eq!(policy.get_victim(0), 2);
}

/// Multiple accesses to the same way should not change victim.
#[test]
fn lru_repeated_access_same_way() {
    let mut policy = LruPolicy::new(1, 4);

    policy.update(0, 0);
    policy.update(0, 1);
    policy.update(0, 2);
    policy.update(0, 3);
    // LRU = 0.

    // Repeated access to way 3 (already MRU) should keep victim the same.
    policy.update(0, 3);
    assert_eq!(policy.get_victim(0), 0);
    policy.update(0, 3);
    assert_eq!(policy.get_victim(0), 0);
}

/// LRU operates independently across sets.
#[test]
fn lru_independent_sets() {
    let mut policy = LruPolicy::new(2, 4);

    // Touch set 0 ways 0..3
    for w in 0..4 {
        policy.update(0, w);
    }
    // Set 0: LRU = 0, Set 1: LRU = 3 (initial).
    assert_eq!(policy.get_victim(0), 0);
    assert_eq!(policy.get_victim(1), 3);

    // Touch set 1 ways in reverse order
    for w in (0..4).rev() {
        policy.update(1, w);
    }
    // Set 1: Stack [0, 1, 2, 3], LRU = 3.
    assert_eq!(policy.get_victim(1), 3);
}

/// 2-way LRU: simplest case.
#[test]
fn lru_two_way() {
    let mut policy = LruPolicy::new(1, 2);

    // Initial: [0, 1]. LRU = 1.
    assert_eq!(policy.get_victim(0), 1);

    policy.update(0, 1);
    // Stack: [1, 0]. LRU = 0.
    assert_eq!(policy.get_victim(0), 0);

    policy.update(0, 0);
    // Stack: [0, 1]. LRU = 1.
    assert_eq!(policy.get_victim(0), 1);
}

// ══════════════════════════════════════════════════════════
// 2. FIFO Policy
// ══════════════════════════════════════════════════════════

/// FIFO pointer starts at 0 and advances through all ways round-robin.
#[test]
fn fifo_round_robin_eviction_order() {
    let mut policy = FifoPolicy::new(1, 4);

    // Simulate 4 fills: get_victim returns current pointer, update advances it.
    assert_eq!(policy.get_victim(0), 0);
    policy.update(0, 0);

    assert_eq!(policy.get_victim(0), 1);
    policy.update(0, 1);

    assert_eq!(policy.get_victim(0), 2);
    policy.update(0, 2);

    assert_eq!(policy.get_victim(0), 3);
    policy.update(0, 3);

    // Wraps around to 0.
    assert_eq!(policy.get_victim(0), 0);
}

/// Accessing a way that is NOT the current pointer does not advance it.
#[test]
fn fifo_access_non_head_ignored() {
    let mut policy = FifoPolicy::new(1, 4);

    // Pointer at 0. Accessing way 2 (a hit) doesn't advance.
    policy.update(0, 2);
    assert_eq!(policy.get_victim(0), 0);

    policy.update(0, 3);
    assert_eq!(policy.get_victim(0), 0);
}

/// Accessing the current pointer advances it (simulates cache fill at that way).
#[test]
fn fifo_access_head_advances_pointer() {
    let mut policy = FifoPolicy::new(1, 4);

    // Pointer at 0. Fill way 0 → pointer moves to 1.
    policy.update(0, 0);
    assert_eq!(policy.get_victim(0), 1);

    // Hit on way 0 again → pointer stays at 1.
    policy.update(0, 0);
    assert_eq!(policy.get_victim(0), 1);
}

/// FIFO wraps correctly for small associativity.
#[test]
fn fifo_wraps_two_way() {
    let mut policy = FifoPolicy::new(1, 2);

    assert_eq!(policy.get_victim(0), 0);
    policy.update(0, 0);
    assert_eq!(policy.get_victim(0), 1);
    policy.update(0, 1);
    assert_eq!(policy.get_victim(0), 0); // wrap
}

// ══════════════════════════════════════════════════════════
// 3. PLRU Policy
// ══════════════════════════════════════════════════════════

/// PLRU: initial state has all bits 0, so victim is way 0 (first unset bit).
#[test]
fn plru_initial_victim_is_zero() {
    let mut policy = PlruPolicy::new(1, 4);
    assert_eq!(policy.get_victim(0), 0);
}

/// PLRU: after accessing way 0, its bit is set; victim shifts to way 1.
#[test]
fn plru_access_protects_way() {
    let mut policy = PlruPolicy::new(1, 4);

    policy.update(0, 0);
    // Bits: 0001. First unset bit is at position 1.
    assert_eq!(policy.get_victim(0), 1);

    policy.update(0, 1);
    // Bits: 0011. First unset bit is at position 2.
    assert_eq!(policy.get_victim(0), 2);

    policy.update(0, 2);
    // Bits: 0111. First unset bit is at position 3.
    assert_eq!(policy.get_victim(0), 3);
}

/// PLRU: when all bits are set (all accessed), reset to only the last-accessed
/// way's bit. Victim should be the first unset after reset.
#[test]
fn plru_wraps_when_all_bits_set() {
    let mut policy = PlruPolicy::new(1, 4);

    policy.update(0, 0);
    policy.update(0, 1);
    policy.update(0, 2);
    // Bits: 0111. Victim = 3.
    assert_eq!(policy.get_victim(0), 3);

    // Access way 3 → all bits become set (1111).
    // Implementation resets to only bit 3 set (1000).
    policy.update(0, 3);
    // Bits after reset: 1000. First unset = 0.
    assert_eq!(policy.get_victim(0), 0);
}

/// PLRU: accessing a previously-accessed way still works correctly.
#[test]
fn plru_reaccess_way() {
    let mut policy = PlruPolicy::new(1, 4);

    policy.update(0, 0);
    policy.update(0, 1);
    // Bits: 0011. Victim = 2.

    // Re-access way 0. Bit 0 was already set, no change.
    policy.update(0, 0);
    // Bits: 0011. Victim = 2.
    assert_eq!(policy.get_victim(0), 2);
}

/// PLRU with 2 ways: toggle behavior.
#[test]
fn plru_two_way() {
    let mut policy = PlruPolicy::new(1, 2);

    // Initial: bits=00, victim=0.
    assert_eq!(policy.get_victim(0), 0);

    policy.update(0, 0);
    // Bits=01. Victim=1.
    assert_eq!(policy.get_victim(0), 1);

    policy.update(0, 1);
    // All bits set (11) → reset to just bit 1 → bits=10. Victim=0.
    assert_eq!(policy.get_victim(0), 0);
}

// ══════════════════════════════════════════════════════════
// 4. MRU Policy
// ══════════════════════════════════════════════════════════

/// MRU: initial victim is way 0 (first in the MRU-ordered stack).
#[test]
fn mru_initial_victim() {
    let mut policy = MruPolicy::new(1, 4);
    // Stack: [0, 1, 2, 3]. MRU = 0.
    assert_eq!(policy.get_victim(0), 0);
}

/// MRU: after accessing a way, it becomes the victim.
#[test]
fn mru_evicts_most_recently_used() {
    let mut policy = MruPolicy::new(1, 4);

    policy.update(0, 2);
    assert_eq!(
        policy.get_victim(0),
        2,
        "MRU should evict the most recently used way"
    );

    policy.update(0, 1);
    assert_eq!(policy.get_victim(0), 1);

    policy.update(0, 3);
    assert_eq!(policy.get_victim(0), 3);
}

/// MRU: sequential accesses always evict the last-touched way.
#[test]
fn mru_sequential_access() {
    let mut policy = MruPolicy::new(1, 4);

    for w in 0..4 {
        policy.update(0, w);
        assert_eq!(policy.get_victim(0), w);
    }
}

/// MRU: opposite of LRU — after accessing 0,1,2,3 the victim is 3 (MRU).
#[test]
fn mru_opposite_of_lru() {
    let mut policy = MruPolicy::new(1, 4);

    policy.update(0, 0);
    policy.update(0, 1);
    policy.update(0, 2);
    policy.update(0, 3);
    // MRU = 3.
    assert_eq!(policy.get_victim(0), 3);

    // Re-access 0. MRU = 0.
    policy.update(0, 0);
    assert_eq!(policy.get_victim(0), 0);
}

// ══════════════════════════════════════════════════════════
// 5. Random Policy
// ══════════════════════════════════════════════════════════

/// Random: all victims must be in range [0, ways).
#[test]
fn random_victim_always_in_range() {
    let ways = 4;
    let mut policy = RandomPolicy::new(1, ways);

    for _ in 0..200 {
        let victim = policy.get_victim(0);
        assert!(
            victim < ways,
            "Victim {} out of range [0, {})",
            victim,
            ways
        );
    }
}

/// Random: different way counts still produce valid indices.
#[test]
fn random_victim_various_way_counts() {
    for ways in [1, 2, 3, 4, 8, 16] {
        let mut policy = RandomPolicy::new(1, ways);
        for _ in 0..50 {
            let victim = policy.get_victim(0);
            assert!(
                victim < ways,
                "ways={}, victim {} out of range",
                ways,
                victim
            );
        }
    }
}

/// Random: update is a no-op; victims should still change (LFSR advances on get_victim).
#[test]
fn random_update_is_noop() {
    let mut policy = RandomPolicy::new(1, 4);

    let v1 = policy.get_victim(0);
    policy.update(0, v1);
    let v2 = policy.get_victim(0);
    // We can't guarantee they differ, but the LFSR should have advanced.
    // Just verify both are in range.
    assert!(v1 < 4);
    assert!(v2 < 4);
}

/// Random: produces more than one distinct value over many calls (not stuck).
#[test]
fn random_not_stuck() {
    let mut policy = RandomPolicy::new(1, 8);
    let mut seen = std::collections::HashSet::new();
    for _ in 0..100 {
        seen.insert(policy.get_victim(0));
    }
    assert!(
        seen.len() > 1,
        "Random policy produced only {} distinct values over 100 calls",
        seen.len()
    );
}
