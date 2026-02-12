//! PLIC (Platform-Level Interrupt Controller) Unit Tests.
//!
//! Verifies priority-based interrupt arbitration, enable/pending logic,
//! threshold filtering, and claim/complete protocol.

use riscv_core::soc::devices::Device;
use riscv_core::soc::devices::plic::Plic;

// ══════════════════════════════════════════════════════════
// 1. Basic identification
// ══════════════════════════════════════════════════════════

#[test]
fn plic_name() {
    let plic = Plic::new(0xC00_0000);
    assert_eq!(plic.name(), "PLIC");
}

#[test]
fn plic_address_range() {
    let plic = Plic::new(0xC00_0000);
    let (base, size) = plic.address_range();
    assert_eq!(base, 0xC00_0000);
    assert_eq!(size, 0x400_0000);
}

// ══════════════════════════════════════════════════════════
// 2. Priority registers
// ══════════════════════════════════════════════════════════

#[test]
fn plic_set_and_read_priority() {
    let mut plic = Plic::new(0);
    // Priority for source 1 is at offset 4
    plic.write_u32(4, 7);
    assert_eq!(plic.read_u32(4), 7);
}

#[test]
fn plic_priority_source_zero_reserved() {
    let mut plic = Plic::new(0);
    // Source 0 priority at offset 0 — exists but is reserved (no interrupt 0)
    plic.write_u32(0, 5);
    assert_eq!(plic.read_u32(0), 5);
}

// ══════════════════════════════════════════════════════════
// 3. Interrupt enable
// ══════════════════════════════════════════════════════════

#[test]
fn plic_enable_and_check_interrupt() {
    let mut plic = Plic::new(0);
    // Set priority for source 1
    plic.write_u32(4, 3);
    // Enable source 1 for context 0
    // Enable register for ctx 0 at 0x2000
    plic.write_u32(0x2000, 1 << 1);
    // Set threshold for ctx 0 to 0
    plic.write_u32(0x200000, 0);

    // Update pending: source 1 active
    plic.update_irqs(1 << 1);

    let (meip, _seip) = plic.check_interrupts();
    assert!(meip, "Machine external interrupt should be pending");
}

// ══════════════════════════════════════════════════════════
// 4. Threshold filtering
// ══════════════════════════════════════════════════════════

#[test]
fn plic_threshold_filters_low_priority() {
    let mut plic = Plic::new(0);
    plic.write_u32(4, 2); // Source 1 priority = 2
    plic.write_u32(0x2000, 1 << 1); // Enable source 1 for ctx 0
    plic.write_u32(0x200000, 5); // Threshold = 5

    plic.update_irqs(1 << 1);
    let (meip, _) = plic.check_interrupts();
    assert!(!meip, "Priority 2 should be filtered by threshold 5");
}

#[test]
fn plic_threshold_zero_allows_all() {
    let mut plic = Plic::new(0);
    plic.write_u32(4, 1); // Source 1 priority = 1
    plic.write_u32(0x2000, 1 << 1);
    plic.write_u32(0x200000, 0); // Threshold = 0

    plic.update_irqs(1 << 1);
    let (meip, _) = plic.check_interrupts();
    assert!(meip, "Threshold 0 should allow priority 1");
}

// ══════════════════════════════════════════════════════════
// 5. Claim/Complete
// ══════════════════════════════════════════════════════════

#[test]
fn plic_claim_returns_highest_priority_id() {
    let mut plic = Plic::new(0);
    // Source 1: priority 3, Source 2: priority 5
    plic.write_u32(4, 3); // source 1
    plic.write_u32(8, 5); // source 2
    plic.write_u32(0x2000, (1 << 1) | (1 << 2)); // enable both for ctx 0
    plic.write_u32(0x200000, 0);

    plic.update_irqs((1 << 1) | (1 << 2));
    plic.check_interrupts();

    // Claim register for ctx 0 at 0x200004
    let claim = plic.read_u32(0x200004);
    assert_eq!(claim, 2, "Should claim source 2 (highest priority)");
}

#[test]
fn plic_claim_clears_pending() {
    let mut plic = Plic::new(0);
    plic.write_u32(4, 3);
    plic.write_u32(0x2000, 1 << 1);
    plic.write_u32(0x200000, 0);

    plic.update_irqs(1 << 1);
    plic.check_interrupts();

    let claim = plic.read_u32(0x200004);
    assert_eq!(claim, 1);

    // Pending should be cleared for source 1 after claim
    let pending = plic.read_u32(0x1000);
    assert_eq!(
        pending & (1 << 1),
        0,
        "Pending bit should be cleared after claim"
    );
}

#[test]
fn plic_complete_clears_claim() {
    let mut plic = Plic::new(0);
    plic.write_u32(4, 3);
    plic.write_u32(0x2000, 1 << 1);
    plic.write_u32(0x200000, 0);
    plic.update_irqs(1 << 1);
    plic.check_interrupts();

    let _claim = plic.read_u32(0x200004);
    // Complete: write claimed ID back to claim register
    plic.write_u32(0x200004, 1);

    // After completion, no pending interrupts
    plic.update_irqs(0);
    let (meip, _) = plic.check_interrupts();
    assert!(!meip, "No interrupts after complete and clear");
}

// ══════════════════════════════════════════════════════════
// 6. No pending → no interrupt
// ══════════════════════════════════════════════════════════

#[test]
fn plic_no_pending_no_interrupt() {
    let mut plic = Plic::new(0);
    plic.write_u32(4, 3);
    plic.write_u32(0x2000, 1 << 1);
    plic.write_u32(0x200000, 0);

    plic.update_irqs(0);
    let (meip, seip) = plic.check_interrupts();
    assert!(!meip);
    assert!(!seip);
}

// ══════════════════════════════════════════════════════════
// 7. Disabled source doesn't trigger
// ══════════════════════════════════════════════════════════

#[test]
fn plic_disabled_source_no_interrupt() {
    let mut plic = Plic::new(0);
    plic.write_u32(4, 3);
    // Don't enable source 1
    plic.write_u32(0x2000, 0);
    plic.write_u32(0x200000, 0);

    plic.update_irqs(1 << 1);
    let (meip, _) = plic.check_interrupts();
    assert!(!meip, "Disabled source should not trigger");
}
