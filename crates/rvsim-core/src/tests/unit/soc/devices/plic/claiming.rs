//! PLIC claim/complete flow tests.

use crate::common::{HartId, PhysAddr};
use crate::soc::devices::plic::Plic;
use crate::tests::support::probe::{read, write};

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
    crate::tests::support::probe::write(
        &mut plic,
        crate::common::PhysAddr::new(0x200000),
        0_u64,
        4,
    ); // threshold 0
    plic.update_irqs(0);
    settle(&mut plic);

    let claim =
        crate::tests::support::probe::read(&mut plic, crate::common::PhysAddr::new(0x200004), 4)
            as u32;
    assert_eq!(claim, 0);
}

#[test]
fn plic_supervisor_context() {
    let mut plic = Plic::new(0, 1);
    // Source 3, priority 4
    crate::tests::support::probe::write(&mut plic, crate::common::PhysAddr::new(12), 4_u64, 4);
    // Enable source 3 for context 1 (supervisor)
    crate::tests::support::probe::write(
        &mut plic,
        crate::common::PhysAddr::new(0x2000 + 0x80),
        (1 << 3) as u64,
        4,
    );
    // Threshold for ctx 1 = 0
    crate::tests::support::probe::write(
        &mut plic,
        crate::common::PhysAddr::new(0x200000 + 0x1000),
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
    crate::tests::support::probe::write(&mut plic, crate::common::PhysAddr::new(4), 3_u64, 4);
    crate::tests::support::probe::write(
        &mut plic,
        crate::common::PhysAddr::new(0x2000),
        (1 << 1) as u64,
        4,
    );
    crate::tests::support::probe::write(
        &mut plic,
        crate::common::PhysAddr::new(0x200000),
        0_u64,
        4,
    );
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
    crate::tests::support::probe::write(&mut plic, crate::common::PhysAddr::new(5 * 4), 2, 4);
    // Enable source 5 for hart 1's M-mode context (context 2) only.
    crate::tests::support::probe::write(
        &mut plic,
        crate::common::PhysAddr::new(0x2000 + 2 * 0x80),
        1 << 5,
        4,
    );
    // Hart 1's S-mode context (context 3) enables source 5 but its threshold blocks it.
    crate::tests::support::probe::write(
        &mut plic,
        crate::common::PhysAddr::new(0x2000 + 3 * 0x80),
        1 << 5,
        4,
    );
    crate::tests::support::probe::write(
        &mut plic,
        crate::common::PhysAddr::new(0x200000 + 3 * 0x1000),
        2,
        4,
    );

    plic.update_irqs(1 << 5);
    settle(&mut plic);

    assert_eq!(plic.hart_lines(HartId::new(0)), crate::soc::devices::plic::ExternalIrqs::default());
    let hart1 = plic.hart_lines(HartId::new(1));
    assert!(hart1.meip, "hart 1 M-mode context is enabled and above threshold");
    assert!(!hart1.seip, "hart 1 S-mode context is blocked by its threshold");

    // Claiming through hart 1's M-mode context returns source 5 and clears it.
    let claim = crate::tests::support::probe::read(
        &mut plic,
        crate::common::PhysAddr::new(0x200000 + 2 * 0x1000 + 4),
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
    assert_eq!(plic.hart_lines(HartId::new(3)), crate::soc::devices::plic::ExternalIrqs::default());
}

#[test]
fn plic_lines_change_three_cycles_after_the_source() {
    let mut plic = Plic::new(0, 1);
    crate::tests::support::probe::write(&mut plic, crate::common::PhysAddr::new(4), 3_u64, 4);
    crate::tests::support::probe::write(&mut plic, crate::common::PhysAddr::new(0x2000), 1 << 1, 4);
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

/// One cycle of the bus driving a level-triggered source: the line's
/// level, then the recomputed outputs.
fn hold_line_high(plic: &mut Plic, source: u32) {
    for _ in 0..4 {
        plic.update_irqs(1 << source);
        plic.check_interrupts();
    }
}

const SOURCE: u32 = 1;
const HART0_M_CLAIM: u64 = 0x20_0004;
const HART1_M_CLAIM: u64 = 0x20_2004;

/// Two harts' M contexts with `SOURCE` enabled at priority 1, its line high.
fn two_harts_sharing_a_raised_source() -> Plic {
    let mut plic = Plic::new(0, 2);
    write(&mut plic, PhysAddr::new(4 * u64::from(SOURCE)), 1, 4);
    write(&mut plic, PhysAddr::new(0x2000), 1 << SOURCE, 4);
    write(&mut plic, PhysAddr::new(0x2000 + 2 * 0x80), 1 << SOURCE, 4);
    hold_line_high(&mut plic, SOURCE);
    plic
}

#[test]
fn a_claimed_level_triggered_source_cannot_be_claimed_again_before_completion() {
    let mut plic = two_harts_sharing_a_raised_source();
    let first = read(&mut plic, PhysAddr::new(HART0_M_CLAIM), 4);

    hold_line_high(&mut plic, SOURCE);
    let second = read(&mut plic, PhysAddr::new(HART1_M_CLAIM), 4);

    assert_eq!((first, second), (u64::from(SOURCE), 0), "(hart 0 claim, hart 1 claim)");
}

#[test]
fn a_source_still_raised_after_completion_can_be_claimed_again() {
    let mut plic = two_harts_sharing_a_raised_source();
    let claimed = read(&mut plic, PhysAddr::new(HART0_M_CLAIM), 4);
    write(&mut plic, PhysAddr::new(HART0_M_CLAIM), claimed, 4);

    hold_line_high(&mut plic, SOURCE);
    let again = read(&mut plic, PhysAddr::new(HART1_M_CLAIM), 4);

    assert_eq!(again, u64::from(SOURCE));
}
