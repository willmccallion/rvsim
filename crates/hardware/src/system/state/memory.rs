//! Virtual-to-physical translation entry point on `SystemState`.
//!
//! Wraps the MMU's event-driven [`Mmu::translate_async`](crate::uarch::mmu::Mmu::translate_async)
//! and PMP checks. Pipeline stages call this; on a TLB hit / direct-mode
//! address the result is immediate, on a TLB miss the caller stashes the
//! returned walk state until the PTE response arrives in its mailbox.

use super::{CoreCtx, Uncore};
use crate::arch::Hart;
use crate::arch::pmp::PmpResult;
use crate::arch::translation::TranslationResult;
use crate::common::{AccessType, PhysAddr, VirtAddr};
use crate::isa::privileged::Trap;
use crate::uarch::CoreUnits;
use crate::uarch::mmu::TranslateOutcome;
use crate::uarch::mmu::ptw::WalkState;

/// Begins (or completes) translation of a virtual address.
pub(super) fn translate(
    core: &mut CoreUnits,
    hart: &Hart,
    uncore: &Uncore,
    vaddr: VirtAddr,
    access: AccessType,
    size: u64,
) -> TranslateOutcome {
    if uncore.direct_mode {
        let paddr = PhysAddr::new(vaddr.val());

        let is_machine = hart.privilege == crate::isa::privileged::PrivilegeMode::Machine;
        let pmp_result = hart.pmp.check(
            paddr.val(),
            size,
            matches!(access, AccessType::Read),
            matches!(access, AccessType::Write),
            matches!(access, AccessType::Fetch),
            is_machine,
        );
        if pmp_result != PmpResult::Allow {
            return TranslateOutcome::Ready(TranslationResult::fault(
                fault_for(access, vaddr.val()),
                0,
            ));
        }

        if !uncore.bus.is_valid_address(paddr) {
            return TranslateOutcome::Ready(TranslationResult::fault(
                fault_for(access, vaddr.val()),
                0,
            ));
        }
        return TranslateOutcome::Ready(TranslationResult::success(paddr, 0));
    }

    let effective_priv = if access != AccessType::Fetch
        && (hart.csrs.mstatus & crate::arch::csr::MSTATUS_MPRV) != 0
    {
        use crate::arch::csr::{MSTATUS_MPP_MASK, MSTATUS_MPP_SHIFT};
        use crate::isa::privileged::PrivilegeMode;
        let mpp = ((hart.csrs.mstatus >> MSTATUS_MPP_SHIFT) & MSTATUS_MPP_MASK) as u8;
        PrivilegeMode::from_u8(mpp)
    } else {
        hart.privilege
    };

    let outcome =
        core.mmu.translate_async(vaddr, access, effective_priv, &hart.csrs, Some(&hart.pmp));

    finalize_outcome(hart, uncore, outcome, vaddr, access, size, effective_priv)
}

/// Resumes a walk that was parked waiting on a PTE response.
pub(super) fn translate_continue(
    core: &mut CoreUnits,
    hart: &Hart,
    uncore: &Uncore,
    state: WalkState,
    raw_pte: u64,
    bus_transit_cycles: u64,
) -> TranslateOutcome {
    let vaddr = state.vaddr;
    let access = state.access;
    let effective_priv = state.privilege;
    // The walk state carries its own size context only for fault reporting;
    // PMP needs the access size. Translation post-checks size against PMP
    // again once the leaf PTE resolves, but the walk itself reads 8 bytes
    // per PTE which is what `start_walk` / `continue_walk` enforce.
    let size = 8u64;
    let outcome =
        core.mmu.continue_walk(state, raw_pte, &hart.csrs, Some(&hart.pmp), bus_transit_cycles);
    finalize_outcome(hart, uncore, outcome, vaddr, access, size, effective_priv)
}

/// Applies the post-translation PMP + bus-address checks shared by the
/// initial translate and walk continuation paths.
fn finalize_outcome(
    hart: &Hart,
    uncore: &Uncore,
    outcome: TranslateOutcome,
    vaddr: VirtAddr,
    access: AccessType,
    size: u64,
    effective_priv: crate::isa::privileged::PrivilegeMode,
) -> TranslateOutcome {
    match outcome {
        TranslateOutcome::Ready(mut result) => {
            if result.trap.is_none() {
                let paddr = result.paddr.val();
                let is_machine = effective_priv == crate::isa::privileged::PrivilegeMode::Machine;
                let pmp_result = hart.pmp.check(
                    paddr,
                    size,
                    matches!(access, AccessType::Read),
                    matches!(access, AccessType::Write),
                    matches!(access, AccessType::Fetch),
                    is_machine,
                );
                if pmp_result != PmpResult::Allow || !uncore.bus.is_valid_address(result.paddr) {
                    result =
                        TranslationResult::fault(fault_for(access, vaddr.val()), result.cycles);
                }
            }
            TranslateOutcome::Ready(result)
        }
        need_pte @ TranslateOutcome::NeedPte { .. } => need_pte,
    }
}

impl CoreCtx<'_> {
    /// Begins (or completes) translation of a virtual address.
    pub fn translate(
        &mut self,
        vaddr: VirtAddr,
        access: AccessType,
        size: u64,
    ) -> TranslateOutcome {
        translate(self.core, self.hart, self.uncore, vaddr, access, size)
    }

    /// Resumes a walk that was parked waiting on a PTE response.
    pub fn translate_continue(
        &mut self,
        state: WalkState,
        raw_pte: u64,
        bus_transit_cycles: u64,
    ) -> TranslateOutcome {
        translate_continue(self.core, self.hart, self.uncore, state, raw_pte, bus_transit_cycles)
    }
}

const fn fault_for(access: AccessType, addr: u64) -> Trap {
    match access {
        AccessType::Fetch => Trap::InstructionAccessFault(addr),
        AccessType::Read => Trap::LoadAccessFault(addr),
        AccessType::Write => Trap::StoreAccessFault(addr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_translate_direct_mode() {
        let mut config = Config::default();
        config.general.direct_mode = true;
        let mut sys = crate::system::SystemState::build(&config, "");
        let mut state = sys.core_ctx(0);

        let result = state.translate(VirtAddr::new(0x8000_0000), AccessType::Read, 4);
        match result {
            TranslateOutcome::Ready(r) => {
                assert_eq!(r.paddr.val(), 0x8000_0000);
                assert!(r.trap.is_none());
            }
            TranslateOutcome::NeedPte { .. } => panic!("direct mode should be Ready"),
        }

        let result = state.translate(VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF), AccessType::Fetch, 4);
        match result {
            TranslateOutcome::Ready(r) => assert!(r.trap.is_some()),
            TranslateOutcome::NeedPte { .. } => panic!("direct mode should be Ready"),
        }
    }
}
