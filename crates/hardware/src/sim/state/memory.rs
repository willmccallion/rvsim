//! Virtual-to-physical translation entry point on `SimState`.
//!
//! Wraps the MMU's event-driven [`Mmu::translate_async`](crate::core::units::mmu::Mmu::translate_async)
//! and PMP checks. Pipeline stages call this; on a TLB hit / direct-mode
//! address the result is immediate, on a TLB miss the caller stashes the
//! returned walk state until the PTE response arrives in its mailbox.

use super::CoreCtx;
use crate::common::{AccessType, PhysAddr, TranslationResult, Trap, VirtAddr};
use crate::core::units::mmu::TranslateOutcome;
use crate::core::units::mmu::pmp::PmpResult;
use crate::core::units::mmu::ptw::WalkState;

/// Outcome of [`SimState::translate`] / [`SimState::translate_continue`].
///
/// Mirrors [`TranslateOutcome`] but lifted onto `SimState` so callers don't
/// import the MMU module directly.
#[derive(Clone, Debug)]
pub enum TranslateResult {
    /// Translation finished (success or fault). The cycle field of the
    /// inner `TranslationResult` carries any PMP / TLB latency that
    /// applies before the access begins.
    Ready(TranslationResult),
    /// Caller must issue a `MemReq` for `pte_addr`, stash `state`, and
    /// resume via [`SimState::translate_continue`] when the response arrives.
    NeedPte {
        /// Address of the next PTE to read.
        pte_addr: PhysAddr,
        /// Walk state to stash until the response arrives.
        state: WalkState,
    },
}

impl CoreCtx<'_> {
    /// Begins (or completes) translation of a virtual address.
    pub fn translate(
        &mut self,
        vaddr: VirtAddr,
        access: AccessType,
        size: u64,
    ) -> TranslateResult {
        if self.direct_mode {
            let paddr = PhysAddr::new(vaddr.val());

            let is_machine = self.hart.privilege == crate::core::arch::mode::PrivilegeMode::Machine;
            let pmp_result = self.hart.pmp.check(
                paddr.val(),
                size,
                matches!(access, AccessType::Read),
                matches!(access, AccessType::Write),
                matches!(access, AccessType::Fetch),
                is_machine,
            );
            if pmp_result != PmpResult::Allow {
                return TranslateResult::Ready(TranslationResult::fault(
                    fault_for(access, vaddr.val()),
                    0,
                ));
            }

            if !self.bus.is_valid_address(paddr) {
                return TranslateResult::Ready(TranslationResult::fault(
                    fault_for(access, vaddr.val()),
                    0,
                ));
            }
            return TranslateResult::Ready(TranslationResult::success(paddr, 0));
        }

        let effective_priv = if access != AccessType::Fetch
            && (self.hart.csrs.mstatus & crate::core::arch::csr::MSTATUS_MPRV) != 0
        {
            use crate::core::arch::csr::{MSTATUS_MPP_MASK, MSTATUS_MPP_SHIFT};
            use crate::core::arch::mode::PrivilegeMode;
            let mpp = ((self.hart.csrs.mstatus >> MSTATUS_MPP_SHIFT) & MSTATUS_MPP_MASK) as u8;
            PrivilegeMode::from_u8(mpp)
        } else {
            self.hart.privilege
        };

        let outcome = self.hart.mmu.translate_async(
            vaddr,
            access,
            effective_priv,
            &self.hart.csrs,
            Some(&self.hart.pmp),
        );

        self.finalize_outcome(outcome, vaddr, access, size, effective_priv)
    }

    /// Resumes a walk that was parked waiting on a PTE response.
    pub fn translate_continue(
        &mut self,
        state: WalkState,
        raw_pte: u64,
        bus_transit_cycles: u64,
    ) -> TranslateResult {
        let vaddr = state.vaddr;
        let access = state.access;
        let effective_priv = state.privilege;
        // The walk state carries its own size context only for fault reporting;
        // PMP needs the access size. Translation post-checks size against PMP
        // again once the leaf PTE resolves, but the walk itself reads 8 bytes
        // per PTE which is what `start_walk` / `continue_walk` enforce.
        let size = 8u64;
        let outcome =
            self.hart.mmu.continue_walk(state, raw_pte, &self.hart.csrs, Some(&self.hart.pmp), bus_transit_cycles);
        self.finalize_outcome(outcome, vaddr, access, size, effective_priv)
    }

    /// Applies the post-translation PMP + bus-address checks shared by the
    /// initial translate and walk continuation paths.
    fn finalize_outcome(
        &self,
        outcome: TranslateOutcome,
        vaddr: VirtAddr,
        access: AccessType,
        size: u64,
        effective_priv: crate::core::arch::mode::PrivilegeMode,
    ) -> TranslateResult {
        match outcome {
            TranslateOutcome::Ready(mut result) => {
                if result.trap.is_none() {
                    let paddr = result.paddr.val();
                    let is_machine =
                        effective_priv == crate::core::arch::mode::PrivilegeMode::Machine;
                    let pmp_result = self.hart.pmp.check(
                        paddr,
                        size,
                        matches!(access, AccessType::Read),
                        matches!(access, AccessType::Write),
                        matches!(access, AccessType::Fetch),
                        is_machine,
                    );
                    if pmp_result != PmpResult::Allow
                        || !self.bus.is_valid_address(result.paddr)
                    {
                        result = TranslationResult::fault(
                            fault_for(access, vaddr.val()),
                            result.cycles,
                        );
                    }
                }
                TranslateResult::Ready(result)
            }
            TranslateOutcome::NeedPte { pte_addr, state } => {
                TranslateResult::NeedPte { pte_addr, state }
            }
        }
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
        let mut sys = crate::sim::SimState::build(&config, "");
        let mut state = sys.core_ctx(0);

        let result = state.translate(VirtAddr::new(0x8000_0000), AccessType::Read, 4);
        match result {
            TranslateResult::Ready(r) => {
                assert_eq!(r.paddr.val(), 0x8000_0000);
                assert!(r.trap.is_none());
            }
            TranslateResult::NeedPte { .. } => panic!("direct mode should be Ready"),
        }

        let result = state.translate(VirtAddr::new(0xFFFF_FFFF_FFFF_FFFF), AccessType::Fetch, 4);
        match result {
            TranslateResult::Ready(r) => assert!(r.trap.is_some()),
            TranslateResult::NeedPte { .. } => panic!("direct mode should be Ready"),
        }
    }
}
