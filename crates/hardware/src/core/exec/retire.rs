//! The architectural effects of retiring an instruction.
//!
//! Shared by every engine that retires instructions: the pipelines' commit
//! stages and the atomic core.

use crate::common::constants::{
    DELEG_MEIP_BIT, DELEG_MSIP_BIT, DELEG_MTIP_BIT, DELEG_SEIP_BIT, DELEG_SSIP_BIT, DELEG_STIP_BIT,
    PAGE_SHIFT, VPN_MASK,
};
use crate::common::{Asid, RegIdx, SfenceVmaInfo, Trap, Vpn};
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::arch::trap::TrapHandler;
use crate::core::units::vpu::shadow::VectorWrites;
use crate::core::units::vpu::types::VectorConfig;
use crate::sim::CoreCtx;

/// Interrupts in the privileged spec's fixed decreasing priority order (MEI,
/// MSI, MTI, SEI, SSI, STI), as `(mip bit, mie bit, mideleg bit)`.
const INTERRUPT_PRIORITY: [(u64, u64, u64); 6] = [
    (csr::MIP_MEIP, csr::MIE_MEIP, 1 << DELEG_MEIP_BIT),
    (csr::MIP_MSIP, csr::MIE_MSIP, 1 << DELEG_MSIP_BIT),
    (csr::MIP_MTIP, csr::MIE_MTIE, 1 << DELEG_MTIP_BIT),
    (csr::MIP_SEIP, csr::MIE_SEIP, 1 << DELEG_SEIP_BIT),
    (csr::MIP_SSIP, csr::MIE_SSIP, 1 << DELEG_SSIP_BIT),
    (csr::MIP_STIP, csr::MIE_STIE, 1 << DELEG_STIP_BIT),
];

impl CoreCtx<'_> {
    /// The interrupt the hart takes now, if any: the highest-priority one
    /// pending, enabled and not masked at the current privilege level.
    #[must_use]
    pub fn pending_interrupt(&self) -> Option<Trap> {
        let csrs = &self.hart.csrs;
        let m_global_ie = (csrs.mstatus & csr::MSTATUS_MIE) != 0;
        let s_global_ie = (csrs.mstatus & csr::MSTATUS_SIE) != 0;
        let privilege = self.hart.privilege;

        let check = |bit: u64, enable_bit: u64, deleg_bit: u64| -> Option<Trap> {
            if (csrs.mip & bit) == 0 || (csrs.mie & enable_bit) == 0 {
                return None;
            }
            let delegated = (csrs.mideleg & deleg_bit) != 0;
            let target = if delegated { PrivilegeMode::Supervisor } else { PrivilegeMode::Machine };
            let below = privilege.to_u8() < target.to_u8();
            let enabled_here = privilege == target
                && match target {
                    PrivilegeMode::Machine => m_global_ie,
                    PrivilegeMode::Supervisor => s_global_ie,
                    PrivilegeMode::User => false,
                };
            (below || enabled_here).then(|| TrapHandler::irq_to_trap(bit))
        };

        // Interrupts destined for M-mode are taken before any destined for S-mode.
        [false, true].into_iter().find_map(|to_supervisor| {
            INTERRUPT_PRIORITY
                .iter()
                .filter(|&&(_, _, deleg_bit)| (csrs.mideleg & deleg_bit != 0) == to_supervisor)
                .find_map(|&(bit, enable_bit, deleg_bit)| check(bit, enable_bit, deleg_bit))
        })
    }

    /// Writes an integer register; writes to x0 are dropped.
    pub const fn retire_int_write(&mut self, rd: RegIdx, value: u64) {
        if !rd.is_zero() {
            self.hart.regs.write(rd, value);
        }
    }

    /// Writes a floating-point register and marks the FP state dirty.
    pub const fn retire_fp_write(&mut self, rd: RegIdx, value: u64) {
        self.hart.regs.write_f(rd, value);
        self.mark_fp_dirty();
    }

    /// Applies a vector instruction's register writes.
    pub fn retire_vector_writes(&mut self, writes: &VectorWrites) {
        writes.apply(self.hart.regs.vpr_mut());
        self.mark_vector_retired();
    }

    /// Marks the vector state dirty and clears `vstart`, as every vector
    /// instruction that completes does.
    pub const fn mark_vector_retired(&mut self) {
        self.hart.csrs.mstatus =
            (self.hart.csrs.mstatus & !csr::MSTATUS_VS) | csr::MSTATUS_VS_DIRTY;
        self.hart.csrs.vstart = 0;
    }

    /// Accrues IEEE exception flags into `fflags`.
    pub const fn accrue_fp_flags(&mut self, flags: u8) {
        if flags != 0 {
            self.hart.csrs.fflags |= flags as u64;
            self.mark_fp_dirty();
        }
    }

    /// Applies what a `vset{i}vl{i}` computed.
    pub const fn retire_vector_config(&mut self, config: VectorConfig) {
        self.hart.csrs.vtype = config.vtype;
        self.hart.csrs.vl = config.vl;
        self.mark_vector_retired();
    }

    /// Retires a WFI: the hart waits when some interrupt is enabled or
    /// pending, and returns `true`. With nothing enabled or pending it
    /// would never wake, so it acts as a no-op instead, as `OpenSBI`'s early
    /// boot relies on.
    pub const fn retire_wfi(&mut self) -> bool {
        let waits = self.hart.csrs.mie != 0 || self.hart.csrs.mip != 0;
        if waits {
            self.hart.wfi_waiting = true;
        }
        waits
    }

    /// Retires a FENCE.I: instruction fetch sees every older store.
    pub fn retire_fence_i(&mut self) {
        self.core.l1_i_cache.invalidate_all();
    }

    /// Retires an SFENCE.VMA: flushes the translations it names and clears
    /// the reservation.
    pub fn retire_sfence_vma(&mut self, info: &SfenceVmaInfo) {
        let vpn = || Vpn::new((info.rs1_val >> PAGE_SHIFT) & VPN_MASK);
        let asid = || Asid::new(info.rs2_val as u16);
        let mmu = &mut self.core.mmu;
        for tlb in [&mut mmu.dtlb, &mut mmu.itlb, &mut mmu.l2_tlb] {
            match (!info.rs1_idx.is_zero(), !info.rs2_idx.is_zero()) {
                (false, false) => tlb.flush(),
                (true, false) => tlb.flush_vaddr(vpn()),
                (false, true) => tlb.flush_asid(asid()),
                (true, true) => tlb.flush_vaddr_asid(vpn(), asid()),
            }
        }
        self.clear_reservation();
    }

    const fn mark_fp_dirty(&mut self) {
        self.hart.csrs.mstatus =
            (self.hart.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn no_interrupt_is_taken_when_none_is_pending() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let state = sys.core_ctx(0);

        assert!(state.pending_interrupt().is_none());
    }

    #[test]
    fn a_pending_enabled_machine_interrupt_is_taken_in_machine_mode() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let state = sys.core_ctx(0);

        state.hart.csrs.mip = csr::MIP_MEIP;
        state.hart.csrs.mie = csr::MIE_MEIP;
        state.hart.csrs.mstatus |= csr::MSTATUS_MIE;
        state.hart.privilege = PrivilegeMode::Machine;

        assert_eq!(state.pending_interrupt(), Some(Trap::MachineExternalInterrupt));
    }

    #[test]
    fn a_delegated_interrupt_is_taken_in_supervisor_mode() {
        let config = Config::default();
        let mut sys = crate::sim::SimState::build(&config, "");
        let state = sys.core_ctx(0);

        state.hart.csrs.mip = csr::MIP_SEIP;
        state.hart.csrs.mie = csr::MIE_SEIP;
        state.hart.csrs.mstatus |= csr::MSTATUS_SIE;
        state.hart.csrs.mideleg |= 1 << DELEG_SEIP_BIT;
        state.hart.privilege = PrivilegeMode::Supervisor;

        assert_eq!(state.pending_interrupt(), Some(Trap::SupervisorExternalInterrupt));
    }
}
