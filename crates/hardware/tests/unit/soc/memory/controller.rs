//! Memory Controller Unit Tests.
//!
//! Verifies SimpleController (fixed latency) and DramController
//! (multi-bank, row-buffer-aware, refresh-capable DRAM timing).

use rvsim_core::soc::memory::controller::{
    DramConfig, DramController, MemoryController, SimpleController,
};

/// Helper: create a DramController with refresh disabled for simpler timing tests.
/// 8 banks, 2048-byte rows, t_cas=5, t_ras=10, t_pre=8, t_rrd=4.
fn dram_no_refresh(t_cas: u64, t_ras: u64, t_pre: u64) -> DramController {
    DramController::new(DramConfig {
        t_cas,
        t_ras,
        t_pre,
        t_rrd: 4,
        num_banks: 8,
        row_size_bytes: 2048,
        t_refi: 0,
        t_rfc: 0,
    })
}

/// Helper: default timing (5/10/8) with refresh disabled.
fn dram_default() -> DramController {
    dram_no_refresh(5, 10, 8)
}

// Address helpers for 8 banks, 2048-byte rows.
// bank = (addr >> 11) % 8
// Addresses in the same bank are separated by 8 * 2048 = 0x4000.

/// Returns an address in the given bank and row-group.
/// bank 0 row 0 = 0x0000, bank 0 row 1 = 0x4000, bank 1 row 0 = 0x0800, etc.
fn addr(bank: usize, row_group: u64) -> u64 {
    (row_group * 8 + bank as u64) * 2048
}

// ══════════════════════════════════════════════════════════
// 1. SimpleController
// ══════════════════════════════════════════════════════════

#[test]
fn simple_controller_fixed_latency() {
    let mut ctrl = SimpleController::new(10);
    assert_eq!(ctrl.access_latency(0x1000, 0), 10);
    assert_eq!(ctrl.access_latency(0x2000, 100), 10);
    assert_eq!(ctrl.access_latency(0x3000, 200), 10);
}

#[test]
fn simple_controller_zero_latency() {
    let mut ctrl = SimpleController::new(0);
    assert_eq!(ctrl.access_latency(0, 0), 0);
}

#[test]
fn simple_controller_address_independent() {
    let mut ctrl = SimpleController::new(5);
    assert_eq!(ctrl.access_latency(0, 0), 5);
    assert_eq!(ctrl.access_latency(u64::MAX, 0), 5);
}

// ══════════════════════════════════════════════════════════
// 2. DramController: Cold start (no row open)
// ══════════════════════════════════════════════════════════

#[test]
fn dram_cold_start_latency() {
    let mut ctrl = dram_default();
    // First access ever: t_ras + t_cas = 10 + 5 = 15
    assert_eq!(ctrl.access_latency(addr(0, 0), 0), 15);
}

// ══════════════════════════════════════════════════════════
// 3. DramController: Row buffer hit (same bank, same row)
// ══════════════════════════════════════════════════════════

#[test]
fn dram_row_buffer_hit() {
    let mut ctrl = dram_default();
    // Open a row in bank 0.
    ctrl.access_latency(addr(0, 0), 0);
    // Second access to same bank, same row → t_cas = 5
    assert_eq!(ctrl.access_latency(addr(0, 0) + 64, 20), 5);
}

#[test]
fn dram_row_buffer_hit_multiple() {
    let mut ctrl = dram_default();
    ctrl.access_latency(addr(0, 0), 0);
    // Multiple accesses within the same bank+row.
    assert_eq!(ctrl.access_latency(addr(0, 0) + 0x100, 20), 5);
    assert_eq!(ctrl.access_latency(addr(0, 0) + 0x200, 40), 5);
    assert_eq!(ctrl.access_latency(addr(0, 0) + 0x300, 60), 5);
}

// ══════════════════════════════════════════════════════════
// 4. DramController: Row buffer miss (same bank, different row)
// ══════════════════════════════════════════════════════════

#[test]
fn dram_row_buffer_miss_same_bank() {
    let mut ctrl = dram_default();
    // Open row 0 in bank 0.
    ctrl.access_latency(addr(0, 0), 0);
    // Access a different row in the SAME bank → pre + ras + cas = 8 + 10 + 5 = 23
    assert_eq!(ctrl.access_latency(addr(0, 1), 100), 23);
}

#[test]
fn dram_row_switch_back_same_bank() {
    let mut ctrl = dram_default();
    ctrl.access_latency(addr(0, 0), 0); // cold open row 0 in bank 0
    ctrl.access_latency(addr(0, 0) + 4, 20); // hit in bank 0
    ctrl.access_latency(addr(0, 1), 100); // miss: switch to row 1 in bank 0
    ctrl.access_latency(addr(0, 1) + 4, 200); // hit in bank 0
    // Switch back to row 0 in bank 0 → miss again.
    assert_eq!(ctrl.access_latency(addr(0, 0), 300), 23);
}

// ══════════════════════════════════════════════════════════
// 5. Multi-bank parallelism: different banks are independent
// ══════════════════════════════════════════════════════════

#[test]
fn dram_different_banks_both_row_hits() {
    let mut ctrl = dram_default();
    // Open row 0 in bank 0.
    ctrl.access_latency(addr(0, 0), 0);
    // Open row 0 in bank 1 (different bank = cold start, not row miss).
    ctrl.access_latency(addr(1, 0), 100);
    // Now both banks have their rows open. Both should get row hits.
    assert_eq!(
        ctrl.access_latency(addr(0, 0) + 64, 200),
        5,
        "bank 0 row hit"
    );
    assert_eq!(
        ctrl.access_latency(addr(1, 0) + 64, 300),
        5,
        "bank 1 row hit"
    );
}

#[test]
fn dram_different_banks_different_rows_no_conflict() {
    let mut ctrl = dram_default();
    // Open different rows in different banks.
    ctrl.access_latency(addr(0, 0), 0); // bank 0, row 0
    ctrl.access_latency(addr(1, 1), 100); // bank 1, row 1
    ctrl.access_latency(addr(2, 2), 200); // bank 2, row 2
    // All three should be row hits (each bank independently tracks its row).
    assert_eq!(ctrl.access_latency(addr(0, 0) + 8, 300), 5, "bank 0 hit");
    assert_eq!(ctrl.access_latency(addr(1, 1) + 8, 400), 5, "bank 1 hit");
    assert_eq!(ctrl.access_latency(addr(2, 2) + 8, 500), 5, "bank 2 hit");
}

// ══════════════════════════════════════════════════════════
// 6. Row boundary
// ══════════════════════════════════════════════════════════

#[test]
fn dram_row_boundary_exact() {
    let mut ctrl = dram_default();
    // Access last byte of row 0 in bank 0.
    ctrl.access_latency(addr(0, 0) + 2047, 0);
    // Access within the same row should hit.
    assert_eq!(ctrl.access_latency(addr(0, 0) + 1024, 100), 5);
}

// ══════════════════════════════════════════════════════════
// 7. Timing parameter variations
// ══════════════════════════════════════════════════════════

#[test]
fn dram_low_latency() {
    let mut ctrl = dram_no_refresh(1, 2, 1);
    assert_eq!(ctrl.access_latency(addr(0, 0), 0), 3); // cold: ras+cas
    assert_eq!(ctrl.access_latency(addr(0, 0), 10), 1); // hit: cas
    assert_eq!(ctrl.access_latency(addr(0, 1), 100), 4); // miss: pre+ras+cas
}

#[test]
fn dram_high_latency() {
    let mut ctrl = dram_no_refresh(20, 40, 30);
    assert_eq!(ctrl.access_latency(addr(0, 0), 0), 60); // cold: 40+20
    assert_eq!(ctrl.access_latency(addr(0, 0), 100), 20); // hit
    assert_eq!(ctrl.access_latency(addr(0, 1), 200), 90); // miss: 30+40+20
}

// ══════════════════════════════════════════════════════════
// 8. Configurable row size
// ══════════════════════════════════════════════════════════

#[test]
fn dram_custom_row_size_4k() {
    // 4 KiB rows, 4 banks, no refresh.
    let mut ctrl = DramController::new(DramConfig {
        t_cas: 5,
        t_ras: 10,
        t_pre: 8,
        t_rrd: 4,
        num_banks: 4,
        row_size_bytes: 4096,
        t_refi: 0,
        t_rfc: 0,
    });
    // row_shift = 12, bank = (addr >> 12) % 4
    // bank 0, row 0: [0x0000, 0x0FFF]
    // bank 0, row 1: [0x4000, 0x4FFF] (stride = 4 * 4096 = 0x4000)
    ctrl.access_latency(0x0000, 0); // cold open bank 0 row 0
    assert_eq!(
        ctrl.access_latency(0x0800, 50),
        5,
        "4KiB row: 0x0800 is same row as 0x0000"
    );
    assert_eq!(
        ctrl.access_latency(0x0FFF, 100),
        5,
        "4KiB row: 0x0FFF still in same row"
    );
}

// ══════════════════════════════════════════════════════════
// 9. Refresh modeling
// ══════════════════════════════════════════════════════════

#[test]
fn dram_refresh_causes_latency_spike() {
    // t_refi=100, t_rfc=20 for easy testing. 2 banks.
    let mut ctrl = DramController::new(DramConfig {
        t_cas: 5,
        t_ras: 10,
        t_pre: 8,
        t_rrd: 4,
        num_banks: 2,
        row_size_bytes: 2048,
        t_refi: 100,
        t_rfc: 20,
    });

    // Open a row before refresh.
    ctrl.access_latency(addr(0, 0), 0);
    // Access before refresh fires — should be row hit.
    assert_eq!(ctrl.access_latency(addr(0, 0) + 8, 50), 5);

    // At cycle 100, refresh fires. All banks busy until cycle 120.
    // Access at cycle 100 should pay refresh wait + cold start (rows are closed).
    let lat = ctrl.access_latency(addr(0, 0), 100);
    // After refresh: effective_cycle=120, then cold start = ras+cas = 15
    // Total = (120 - 100) + 10 + 5 = 35
    assert!(lat > 5, "refresh should add extra latency, got {}", lat);
}

#[test]
fn dram_refresh_periodic() {
    // Verify refresh fires periodically.
    let mut ctrl = DramController::new(DramConfig {
        t_cas: 5,
        t_ras: 10,
        t_pre: 8,
        t_rrd: 4,
        num_banks: 2,
        row_size_bytes: 2048,
        t_refi: 100,
        t_rfc: 20,
    });

    // Before first refresh: row hit.
    ctrl.access_latency(addr(0, 0), 0);
    assert_eq!(ctrl.access_latency(addr(0, 0) + 8, 50), 5);

    // During first refresh window (cycle 100-120): extra latency.
    let lat1 = ctrl.access_latency(addr(0, 0), 105);
    assert!(lat1 > 5, "first refresh should cause spike, got {}", lat1);

    // Re-open the row after first refresh, then access normally.
    ctrl.access_latency(addr(0, 0), 150);
    assert_eq!(ctrl.access_latency(addr(0, 0) + 8, 170), 5);

    // Second refresh at cycle 200: another spike.
    let lat2 = ctrl.access_latency(addr(0, 0), 200);
    assert!(lat2 > 5, "second refresh should cause spike, got {}", lat2);
}

#[test]
fn dram_no_refresh_when_disabled() {
    // t_refi=0 disables refresh.
    let mut ctrl = DramController::new(DramConfig {
        t_cas: 5,
        t_ras: 10,
        t_pre: 8,
        t_rrd: 4,
        num_banks: 2,
        row_size_bytes: 2048,
        t_refi: 0,
        t_rfc: 350,
    });

    ctrl.access_latency(addr(0, 0), 0);
    // Even at very high cycle counts, no refresh penalty.
    assert_eq!(ctrl.access_latency(addr(0, 0) + 8, 100_000), 5);
}

// ══════════════════════════════════════════════════════════
// 10. tRRD enforcement
// ══════════════════════════════════════════════════════════

#[test]
fn dram_trrd_back_to_back_activations() {
    // t_rrd=4: minimum 4 cycles between activations to different banks.
    let mut ctrl = dram_default();
    // First activation at cycle 0 → bank 0.
    let lat1 = ctrl.access_latency(addr(0, 0), 0);
    assert_eq!(lat1, 15); // cold start: ras+cas

    // Immediate second activation at cycle 0 to bank 1.
    // tRRD forces waiting: earliest activate = 0 + 4 = 4.
    // latency = (4 - 0) + ras + cas = 4 + 10 + 5 = 19
    let lat2 = ctrl.access_latency(addr(1, 0), 0);
    assert_eq!(lat2, 19, "tRRD should delay the second bank activation");
}

#[test]
fn dram_trrd_not_applied_to_row_hits() {
    let mut ctrl = dram_default();
    // Open rows in two banks with sufficient spacing.
    ctrl.access_latency(addr(0, 0), 0);
    ctrl.access_latency(addr(1, 0), 100);
    // Row hits don't activate, so tRRD shouldn't apply.
    assert_eq!(ctrl.access_latency(addr(0, 0) + 8, 200), 5);
    assert_eq!(ctrl.access_latency(addr(1, 0) + 8, 200), 5);
}
