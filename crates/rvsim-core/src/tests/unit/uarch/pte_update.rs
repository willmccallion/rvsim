//! The hardware A/D update a walk's access applies to its leaf PTE, which
//! happens only while the PTE still holds what the walk checked.

use crate::arch::translation::PteUpdate;
use crate::common::PhysAddr;
use crate::config::defaults::RAM_BASE;
use crate::system::SystemState;
use crate::tests::support::harness::minimal_system;
use crate::uarch::ctx::stage::PteUpdateOutcome;

const PTE_ADDR: u64 = RAM_BASE + 0x1000;
const PTE_V_R_W: u64 = 0b111;
const PTE_A: u64 = 1 << 6;
const PTE_D: u64 = 1 << 7;
const PTE_FRAME: u64 = 0x8_0400 << 10;

fn update_setting_a_and_d(pte_addr: u64) -> PteUpdate {
    PteUpdate {
        pte_addr: PhysAddr::new(pte_addr),
        walked_pte: PTE_FRAME | PTE_V_R_W,
        set_bits: PTE_A | PTE_D,
    }
}

fn system_holding_pte(pte: u64) -> SystemState {
    let mut sys = minimal_system();
    sys.uncore.kernel.memory.load(PhysAddr::new(PTE_ADDR), &pte.to_le_bytes());
    sys
}

fn stored_pte(sys: &SystemState) -> Option<u64> {
    sys.uncore.kernel.memory.read(PhysAddr::new(PTE_ADDR), 8)
}

#[test]
fn an_update_whose_bits_are_already_set_is_reported_as_already_set() {
    let already_set = PTE_FRAME | PTE_V_R_W | PTE_A | PTE_D;
    let mut sys = system_holding_pte(already_set);
    let mut core = sys.core_ctx(0);

    let outcome = core.stage().apply_pte_update(&update_setting_a_and_d(PTE_ADDR));

    assert_eq!(outcome, PteUpdateOutcome::AlreadySet);
}

#[test]
fn an_update_sets_the_missing_bits_and_reports_the_pte_it_wrote() {
    let mut sys = system_holding_pte(PTE_FRAME | PTE_V_R_W | PTE_A);
    let mut core = sys.core_ctx(0);

    let outcome = core.stage().apply_pte_update(&update_setting_a_and_d(PTE_ADDR));

    let updated = PTE_FRAME | PTE_V_R_W | PTE_A | PTE_D;
    assert_eq!(outcome, PteUpdateOutcome::Written(updated));
    assert_eq!(stored_pte(&sys), Some(updated));
}

#[test]
fn an_update_to_a_pte_that_changed_beyond_its_ad_bits_is_refused() {
    let remapped = (PTE_FRAME + (1 << 10)) | PTE_V_R_W;
    let mut sys = system_holding_pte(remapped);
    let mut core = sys.core_ctx(0);

    let outcome = core.stage().apply_pte_update(&update_setting_a_and_d(PTE_ADDR));

    assert_eq!(outcome, PteUpdateOutcome::Changed);
    assert_eq!(stored_pte(&sys), Some(remapped));
}
