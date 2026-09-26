//! PLIC claim/complete flow tests.

use rvsim_core::common::HartId;
use rvsim_core::soc::devices::plic::Plic;

/// Recomputes the PLIC's outputs and lets them reach the harts: a change
/// becomes visible three cycles after it is computed.
fn settle(plic: &mut Plic) {
    for _ in 0..4 {
        plic.check_interrupts();
    }
}

#[test]
fn plic_claim_with_no_pending_returns_zero() {
    let mut plic = Plic::new(0, 1);
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x200000), 0_u64, 4); // threshold 0
    plic.update_irqs(0);
    settle(&mut plic);

    let claim =
        crate::common::probe::read(&mut plic, rvsim_core::common::PhysAddr::new(0x200004), 4)
            as u32;
    assert_eq!(claim, 0);
}

#[test]
fn plic_supervisor_context() {
    let mut plic = Plic::new(0, 1);
    // Source 3, priority 4
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(12), 4_u64, 4);
    // Enable source 3 for context 1 (supervisor)
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x2000 + 0x80),
        (1 << 3) as u64,
        4,
    );
    // Threshold for ctx 1 = 0
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x200000 + 0x1000),
        0_u64,
        4,
    );

    plic.update_irqs(1 << 3);
    settle(&mut plic);
    let lines = plic.hart_lines(HartId::new(0));
    let (meip, seip) = (lines.meip, lines.seip);
    assert!(!meip, "Not enabled for machine context");
    assert!(seip, "Should trigger supervisor external interrupt");
}

#[test]
fn plic_lines_follow_check_interrupts() {
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
    assert!(plic.hart_lines(HartId::new(0)).meip, "line asserted while the source is pending");

    plic.update_irqs(0);
    settle(&mut plic);
    assert!(!plic.hart_lines(HartId::new(0)).meip, "line drops once the source is gone");
}

#[test]
fn plic_contexts_of_second_hart_are_independent() {
    let mut plic = Plic::new(0, 2);
    // Source 5, priority 2.
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(5 * 4), 2, 4);
    // Enable source 5 for hart 1's M-mode context (context 2) only.
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x2000 + 2 * 0x80),
        1 << 5,
        4,
    );
    // Hart 1's S-mode context (context 3) enables source 5 but its threshold blocks it.
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x2000 + 3 * 0x80),
        1 << 5,
        4,
    );
    crate::common::probe::write(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x200000 + 3 * 0x1000),
        2,
        4,
    );

    plic.update_irqs(1 << 5);
    settle(&mut plic);

    assert_eq!(
        plic.hart_lines(HartId::new(0)),
        rvsim_core::soc::devices::plic::ExternalIrqs::default()
    );
    let hart1 = plic.hart_lines(HartId::new(1));
    assert!(hart1.meip, "hart 1 M-mode context is enabled and above threshold");
    assert!(!hart1.seip, "hart 1 S-mode context is blocked by its threshold");

    // Claiming through hart 1's M-mode context returns source 5 and clears it.
    let claim = crate::common::probe::read(
        &mut plic,
        rvsim_core::common::PhysAddr::new(0x200000 + 2 * 0x1000 + 4),
        4,
    );
    assert_eq!(claim, 5);
    settle(&mut plic);
    assert!(!plic.hart_lines(HartId::new(1)).meip);
}

#[test]
fn plic_lines_of_absent_hart_are_clear() {
    let mut plic = Plic::new(0, 1);
    plic.update_irqs(1 << 1);
    settle(&mut plic);
    assert_eq!(
        plic.hart_lines(HartId::new(3)),
        rvsim_core::soc::devices::plic::ExternalIrqs::default()
    );
}

#[test]
fn plic_lines_change_three_cycles_after_the_source() {
    let mut plic = Plic::new(0, 1);
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(4), 3_u64, 4);
    crate::common::probe::write(&mut plic, rvsim_core::common::PhysAddr::new(0x2000), 1 << 1, 4);
    plic.update_irqs(1 << 1);

    for cycle in 0..3 {
        plic.check_interrupts();
        assert!(
            !plic.hart_lines(HartId::new(0)).meip,
            "still on its way after {} cycles",
            cycle + 1
        );
    }
    plic.check_interrupts();

    assert!(plic.hart_lines(HartId::new(0)).meip, "visible on the fourth cycle");
}
