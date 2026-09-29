//! PLIC (Platform-Level Interrupt Controller) Unit Tests.
//!
//! Verifies priority-based interrupt arbitration, enable/pending logic,
//! threshold filtering, and claim/complete protocol.

use rvsim_core::common::HartId;
use rvsim_core::soc::devices::Device;
use rvsim_core::soc::devices::plic::Plic;

/// Recomputes the PLIC's outputs and lets them reach the harts: a change
/// becomes visible three cycles after it is computed.
fn settle(plic: &mut Plic) {
    for _ in 0..4 {
        plic.check_interrupts();
    }
}

#[test]
fn plic_name() {
    let plic = Plic::new(0xC00_0000, 1);
    assert_eq!(plic.name(), "PLIC");
}

#[test]
fn plic_address_range() {
    let plic = Plic::new(0xC00_0000, 1);
    let (base, size) = plic.address_range();
    assert_eq!(base, 0xC00_0000);
    assert_eq!(size, 0x400_0000);
}

#[test]
fn plic_set_and_read_priority() {
    let mut plic = Plic::new(0, 1);
    // Priority for source 1 is at offset 4
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(4), 7_u64, 4);
    assert_eq!(
        (crate::common::probe::read(&mut plic, rvsim_core::common::PhysAddr::new(4), 4) as u32),
        7
    );
}

#[test]
fn plic_priority_source_zero_reserved() {
    let mut plic = Plic::new(0, 1);
    // Source 0 priority at offset 0 — exists but is reserved (no interrupt 0)
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0), 5_u64, 4);
    assert_eq!(
        (crate::common::probe::read(&mut plic, rvsim_core::common::PhysAddr::new(0), 4) as u32),
        5
    );
}

#[test]
fn plic_enable_and_check_interrupt() {
    let mut plic = Plic::new(0, 1);
    // Set priority for source 1
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(4), 3_u64, 4);
    // Enable source 1 for ctx 0 (enable register at 0x2000).
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x2000),
        (1 << 1) as u64,
        4,
    );
    // Set threshold for ctx 0 to 0
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x200000), 0_u64, 4);

    // Update pending: source 1 active
    plic.update_irqs(1 << 1);

    settle(&mut plic);
    let meip = plic.hart_lines(HartId::new(0)).meip;
    assert!(meip, "Machine external interrupt should be pending");
}

#[test]
fn plic_threshold_filters_low_priority() {
    let mut plic = Plic::new(0, 1);
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(4), 2_u64, 4); // Source 1 priority = 2
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x2000),
        (1 << 1) as u64,
        4,
    ); // Enable source 1 for ctx 0
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x200000), 5_u64, 4); // Threshold = 5

    plic.update_irqs(1 << 1);
    settle(&mut plic);
    let meip = plic.hart_lines(HartId::new(0)).meip;
    assert!(!meip, "Priority 2 should be filtered by threshold 5");
}

#[test]
fn plic_threshold_zero_allows_all() {
    let mut plic = Plic::new(0, 1);
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(4), 1_u64, 4); // Source 1 priority = 1
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x2000),
        (1 << 1) as u64,
        4,
    );
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x200000), 0_u64, 4); // Threshold = 0

    plic.update_irqs(1 << 1);
    settle(&mut plic);
    let meip = plic.hart_lines(HartId::new(0)).meip;
    assert!(meip, "Threshold 0 should allow priority 1");
}

#[test]
fn plic_claim_returns_highest_priority_id() {
    let mut plic = Plic::new(0, 1);
    // Source 1: priority 3, Source 2: priority 5
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(4), 3_u64, 4); // source 1
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(8), 5_u64, 4); // source 2
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x2000),
        ((1 << 1) | (1 << 2)) as u64,
        4,
    ); // enable both for ctx 0
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x200000), 0_u64, 4);

    plic.update_irqs((1 << 1) | (1 << 2));
    settle(&mut plic);

    // Claim register for ctx 0 at 0x200004
    let claim =
        crate::common::probe::read(&mut plic, rvsim_core::common::PhysAddr::new(0x200004), 4)
            as u32;
    assert_eq!(claim, 2, "Should claim source 2 (highest priority)");
}

#[test]
fn plic_claim_clears_pending() {
    let mut plic = Plic::new(0, 1);
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(4), 3_u64, 4);
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x2000),
        (1 << 1) as u64,
        4,
    );
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x200000), 0_u64, 4);

    plic.update_irqs(1 << 1);
    settle(&mut plic);

    let claim =
        crate::common::probe::read(&mut plic, rvsim_core::common::PhysAddr::new(0x200004), 4)
            as u32;
    assert_eq!(claim, 1);

    // Pending should be cleared for source 1 after claim
    let pending =
        crate::common::probe::read(&mut plic, rvsim_core::common::PhysAddr::new(0x1000), 4) as u32;
    assert_eq!(pending & (1 << 1), 0, "Pending bit should be cleared after claim");
}

#[test]
fn plic_complete_clears_claim() {
    let mut plic = Plic::new(0, 1);
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(4), 3_u64, 4);
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x2000),
        (1 << 1) as u64,
        4,
    );
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x200000), 0_u64, 4);
    plic.update_irqs(1 << 1);
    settle(&mut plic);

    let _claim =
        crate::common::probe::read(&mut plic, rvsim_core::common::PhysAddr::new(0x200004), 4)
            as u32;
    // Complete: write claimed ID back to claim register
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x200004), 1_u64, 4);

    // After completion, no pending interrupts
    plic.update_irqs(0);
    settle(&mut plic);
    let meip = plic.hart_lines(HartId::new(0)).meip;
    assert!(!meip, "No interrupts after complete and clear");
}

#[test]
fn plic_no_pending_no_interrupt() {
    let mut plic = Plic::new(0, 1);
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(4), 3_u64, 4);
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x2000),
        (1 << 1) as u64,
        4,
    );
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x200000), 0_u64, 4);

    plic.update_irqs(0);
    settle(&mut plic);
    let lines = plic.hart_lines(HartId::new(0));
    let (meip, seip) = (lines.meip, lines.seip);
    assert!(!meip);
    assert!(!seip);
}

#[test]
fn plic_disabled_source_no_interrupt() {
    let mut plic = Plic::new(0, 1);
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(4), 3_u64, 4);
    // Don't enable source 1
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x2000), 0_u64, 4);
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x200000), 0_u64, 4);

    plic.update_irqs(1 << 1);
    settle(&mut plic);
    let meip = plic.hart_lines(HartId::new(0)).meip;
    assert!(!meip, "Disabled source should not trigger");
}
