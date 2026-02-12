//! Return Address Stack (RAS) Tests.
//!
//! Verifies push/pop/top semantics, overflow behaviour, underflow safety,
//! and correct LIFO ordering for return address prediction.

use riscv_core::core::units::bru::ras::Ras;

// ══════════════════════════════════════════════════════════
// 1. Basic push/pop
// ══════════════════════════════════════════════════════════

#[test]
fn push_pop_single() {
    let mut ras = Ras::new(8);
    ras.push(0x1000);
    assert_eq!(ras.pop(), Some(0x1000));
}

#[test]
fn push_pop_lifo_order() {
    let mut ras = Ras::new(8);
    ras.push(0xA);
    ras.push(0xB);
    ras.push(0xC);
    assert_eq!(ras.pop(), Some(0xC), "Most recent push comes out first");
    assert_eq!(ras.pop(), Some(0xB));
    assert_eq!(ras.pop(), Some(0xA));
}

#[test]
fn push_pop_interleaved() {
    let mut ras = Ras::new(8);
    ras.push(0x100);
    ras.push(0x200);
    assert_eq!(ras.pop(), Some(0x200));
    ras.push(0x300);
    assert_eq!(ras.pop(), Some(0x300));
    assert_eq!(ras.pop(), Some(0x100));
}

// ══════════════════════════════════════════════════════════
// 2. Top (peek) without modifying state
// ══════════════════════════════════════════════════════════

#[test]
fn top_returns_without_removing() {
    let mut ras = Ras::new(8);
    ras.push(0xAAAA);
    assert_eq!(ras.top(), Some(0xAAAA));
    assert_eq!(ras.top(), Some(0xAAAA), "top() must not consume the entry");
    assert_eq!(
        ras.pop(),
        Some(0xAAAA),
        "pop() should still return the value"
    );
}

#[test]
fn top_on_empty_returns_none() {
    let ras = Ras::new(8);
    assert_eq!(ras.top(), None);
}

// ══════════════════════════════════════════════════════════
// 3. Underflow safety
// ══════════════════════════════════════════════════════════

#[test]
fn pop_empty_returns_none() {
    let mut ras = Ras::new(8);
    assert_eq!(ras.pop(), None, "Popping empty RAS must return None");
}

#[test]
fn pop_beyond_pushes_returns_none() {
    let mut ras = Ras::new(8);
    ras.push(0x1);
    assert_eq!(ras.pop(), Some(0x1));
    assert_eq!(ras.pop(), None, "No more entries");
    assert_eq!(ras.pop(), None, "Still empty");
}

#[test]
fn multiple_pop_on_empty() {
    let mut ras = Ras::new(4);
    for _ in 0..10 {
        assert_eq!(ras.pop(), None);
    }
}

// ══════════════════════════════════════════════════════════
// 4. Overflow behaviour
// ══════════════════════════════════════════════════════════

#[test]
fn overflow_overwrites_top() {
    // With capacity 4, pushing 5 entries should overwrite the last slot.
    let mut ras = Ras::new(4);
    ras.push(0xA);
    ras.push(0xB);
    ras.push(0xC);
    ras.push(0xD); // fills to capacity
    ras.push(0xE); // overflow: overwrites top (slot capacity-1)

    // The top should now be the overwritten value
    assert_eq!(ras.pop(), Some(0xE), "Overflow overwrites top entry");
    // What's under it depends on implementation; the important thing is
    // the most recent address is accessible.
}

#[test]
fn capacity_1_always_holds_latest() {
    let mut ras = Ras::new(1);
    ras.push(0x100);
    assert_eq!(ras.top(), Some(0x100));
    ras.push(0x200); // overflow with capacity=1
    // After overflow, pointer stays at capacity, so pop decrements to get the overwritten value
    assert_eq!(ras.pop(), Some(0x200));
}

// ══════════════════════════════════════════════════════════
// 5. Realistic call/return patterns
// ══════════════════════════════════════════════════════════

#[test]
fn nested_calls() {
    // main calls A, A calls B, B calls C.
    // Returns should unwind in reverse: C→B, B→A, A→main.
    let mut ras = Ras::new(16);

    ras.push(0x1004); // main→A return addr
    ras.push(0x2008); // A→B return addr
    ras.push(0x300C); // B→C return addr

    assert_eq!(ras.pop(), Some(0x300C), "Return from C to B");
    assert_eq!(ras.pop(), Some(0x2008), "Return from B to A");
    assert_eq!(ras.pop(), Some(0x1004), "Return from A to main");
}

#[test]
fn recursive_calls() {
    let mut ras = Ras::new(8);
    // Recursive function pushes the same return address multiple times
    for i in 0..5 {
        ras.push(0x4000 + i * 4);
    }
    for i in (0..5).rev() {
        assert_eq!(ras.pop(), Some(0x4000 + i * 4));
    }
}

// ══════════════════════════════════════════════════════════
// 6. Edge cases
// ══════════════════════════════════════════════════════════

#[test]
fn push_pop_at_exactly_capacity() {
    let cap = 4;
    let mut ras = Ras::new(cap);
    for i in 0..cap {
        ras.push(i as u64);
    }
    for i in (0..cap).rev() {
        assert_eq!(ras.pop(), Some(i as u64));
    }
    assert_eq!(ras.pop(), None);
}

#[test]
fn push_zero_address() {
    let mut ras = Ras::new(4);
    ras.push(0);
    assert_eq!(ras.pop(), Some(0), "Zero is a valid return address");
}

#[test]
fn push_max_address() {
    let mut ras = Ras::new(4);
    ras.push(u64::MAX);
    assert_eq!(ras.pop(), Some(u64::MAX));
}
