//! Memory Controller Unit Tests.
//!
//! Verifies SimpleController (fixed latency) and DramController
//! (row-buffer-aware latency with CAS/RAS/precharge).

use riscv_core::soc::memory::controller::{DramController, MemoryController, SimpleController};

// ══════════════════════════════════════════════════════════
// 1. SimpleController
// ══════════════════════════════════════════════════════════

#[test]
fn simple_controller_fixed_latency() {
    let mut ctrl = SimpleController::new(10);
    assert_eq!(ctrl.access_latency(0x1000), 10);
    assert_eq!(ctrl.access_latency(0x2000), 10);
    assert_eq!(ctrl.access_latency(0x3000), 10);
}

#[test]
fn simple_controller_zero_latency() {
    let mut ctrl = SimpleController::new(0);
    assert_eq!(ctrl.access_latency(0), 0);
}

#[test]
fn simple_controller_address_independent() {
    let mut ctrl = SimpleController::new(5);
    // Same latency regardless of address
    assert_eq!(ctrl.access_latency(0), 5);
    assert_eq!(ctrl.access_latency(u64::MAX), 5);
}

// ══════════════════════════════════════════════════════════
// 2. DramController: Cold start (no row open)
// ══════════════════════════════════════════════════════════

#[test]
fn dram_cold_start_latency() {
    let mut ctrl = DramController::new(5, 10, 8);
    // First access: no row open → t_ras + t_cas = 10 + 5 = 15
    assert_eq!(ctrl.access_latency(0x1000), 15);
}

// ══════════════════════════════════════════════════════════
// 3. DramController: Row buffer hit
// ══════════════════════════════════════════════════════════

#[test]
fn dram_row_buffer_hit() {
    let mut ctrl = DramController::new(5, 10, 8);
    // First access opens row
    ctrl.access_latency(0x1000);
    // Second access to same row → t_cas = 5
    assert_eq!(ctrl.access_latency(0x1004), 5);
}

#[test]
fn dram_row_buffer_hit_multiple() {
    let mut ctrl = DramController::new(5, 10, 8);
    ctrl.access_latency(0x2000);
    // Multiple accesses within the same row (row_mask = !2047, so row = addr & ~0x7FF)
    // 0x2000 and 0x2100 are in the same row (both & !0x7FF = 0x2000)
    assert_eq!(ctrl.access_latency(0x2100), 5);
    assert_eq!(ctrl.access_latency(0x2200), 5);
    assert_eq!(ctrl.access_latency(0x27FF), 5);
}

// ══════════════════════════════════════════════════════════
// 4. DramController: Row buffer miss (different row)
// ══════════════════════════════════════════════════════════

#[test]
fn dram_row_buffer_miss() {
    let mut ctrl = DramController::new(5, 10, 8);
    // First access: cold start
    ctrl.access_latency(0x1000);
    // Access different row → t_pre + t_ras + t_cas = 8 + 10 + 5 = 23
    assert_eq!(ctrl.access_latency(0x2800), 23);
}

#[test]
fn dram_row_switch_back() {
    let mut ctrl = DramController::new(5, 10, 8);
    ctrl.access_latency(0x1000); // cold: 15
    ctrl.access_latency(0x1004); // hit: 5
    ctrl.access_latency(0x2800); // miss: 23
    ctrl.access_latency(0x2804); // hit: 5
    assert_eq!(ctrl.access_latency(0x1000), 23); // miss again
}

// ══════════════════════════════════════════════════════════
// 5. DramController: Row boundary
// ══════════════════════════════════════════════════════════

#[test]
fn dram_row_boundary_exact() {
    let mut ctrl = DramController::new(5, 10, 8);
    // row_mask = !2047 = 0xFFFF_FFFF_FFFF_F800
    // Row 0: [0x0000, 0x07FF]
    // Row 1: [0x0800, 0x0FFF]
    ctrl.access_latency(0x07FF); // row 0
    assert_eq!(
        ctrl.access_latency(0x0800),
        23,
        "0x0800 should be a different row"
    );
}

// ══════════════════════════════════════════════════════════
// 6. DramController: Timing parameter variations
// ══════════════════════════════════════════════════════════

#[test]
fn dram_low_latency() {
    let mut ctrl = DramController::new(1, 2, 1);
    assert_eq!(ctrl.access_latency(0), 3); // cold: ras+cas = 3
    assert_eq!(ctrl.access_latency(0), 1); // hit: cas = 1
    assert_eq!(ctrl.access_latency(0x1000), 4); // miss: pre+ras+cas = 4
}

#[test]
fn dram_high_latency() {
    let mut ctrl = DramController::new(20, 40, 30);
    assert_eq!(ctrl.access_latency(0), 60); // cold: 40+20
    assert_eq!(ctrl.access_latency(0), 20); // hit
    assert_eq!(ctrl.access_latency(0x1000), 90); // miss: 30+40+20
}
