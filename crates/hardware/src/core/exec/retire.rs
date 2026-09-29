//! The architectural effects of retiring an instruction.
//!
//! Shared by every engine that retires instructions: the pipelines' commit
//! stages and the atomic core. Each function changes only what it is
//! given.

use crate::common::constants::{
    DELEG_MEIP_BIT, DELEG_MSIP_BIT, DELEG_MTIP_BIT, DELEG_SEIP_BIT, DELEG_SSIP_BIT, DELEG_STIP_BIT,
    PAGE_SHIFT, VPN_MASK,
};
use crate::common::{Asid, SfenceVmaInfo, Trap, Vpn};
use crate::core::Hart;
use crate::core::arch::csr;
use crate::core::arch::trap::TrapHandler;
use crate::core::units::cache::Cache;
use crate::core::units::mmu::Mmu;
use crate::core::units::vpu::shadow::VectorWrites;
use crate::isa::privileged::mode::PrivilegeMode;
use crate::isa::reg::RegIdx;
use crate::isa::rvv::VectorConfig;

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

/// The interrupt `hart` takes now, if any: the highest-priority one
/// pending, enabled and not masked at its privilege level.
#[must_use]
pub fn pending_interrupt(hart: &Hart) -> Option<Trap> {
    let csrs = &hart.csrs;
    let m_global_ie = (csrs.mstatus & csr::MSTATUS_MIE) != 0;
    let s_global_ie = (csrs.mstatus & csr::MSTATUS_SIE) != 0;
    let privilege = hart.privilege;

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
pub const fn write_int(hart: &mut Hart, rd: RegIdx, value: u64) {
    if !rd.is_zero() {
        hart.regs.write(rd, value);
    }
}

/// Writes a floating-point register and marks the FP state dirty.
pub const fn write_fp(hart: &mut Hart, rd: RegIdx, value: u64) {
    hart.regs.write_f(rd, value);
    mark_fp_dirty(hart);
}

/// Applies a vector instruction's register writes.
pub fn apply_vector_writes(hart: &mut Hart, writes: &VectorWrites) {
    writes.apply(hart.regs.vpr_mut());
    mark_vector_retired(hart);
}

/// Marks the vector state dirty and clears `vstart`, as every vector
/// instruction that completes does.
pub const fn mark_vector_retired(hart: &mut Hart) {
    hart.csrs.mstatus = (hart.csrs.mstatus & !csr::MSTATUS_VS) | csr::MSTATUS_VS_DIRTY;
    hart.csrs.vstart = 0;
}

/// Accrues IEEE exception flags into `fflags`.
pub const fn accrue_fp_flags(hart: &mut Hart, flags: u8) {
    if flags != 0 {
        hart.csrs.fflags |= flags as u64;
        mark_fp_dirty(hart);
    }
}

/// Applies what a `vset{i}vl{i}` computed.
pub const fn apply_vector_config(hart: &mut Hart, config: VectorConfig) {
    hart.csrs.vtype = config.vtype;
    hart.csrs.vl = config.vl;
    mark_vector_retired(hart);
}

/// Retires a WFI; returns whether the hart now waits.
///
/// It waits when some interrupt is enabled or pending. With nothing enabled
/// or pending it would never wake, so it acts as a no-op instead, as
/// `OpenSBI`'s early boot relies on.
pub const fn wfi(hart: &mut Hart) -> bool {
    let waits = hart.csrs.mie != 0 || hart.csrs.mip != 0;
    if waits {
        hart.wfi_waiting = true;
    }
    waits
}

/// Retires a FENCE.I: instruction fetch sees every older store.
pub fn fence_i(l1_i_cache: &mut Cache) {
    l1_i_cache.invalidate_all();
}

/// Retires an SFENCE.VMA: flushes the translations it names. Retiring one
/// also clears the hart's reservation, which lives with memory.
pub fn sfence_vma(mmu: &mut Mmu, info: &SfenceVmaInfo) {
    let vpn = || Vpn::new((info.rs1_val >> PAGE_SHIFT) & VPN_MASK);
    let asid = || Asid::new(info.rs2_val as u16);
    for tlb in [&mut mmu.dtlb, &mut mmu.itlb, &mut mmu.l2_tlb] {
        match (!info.rs1_idx.is_zero(), !info.rs2_idx.is_zero()) {
            (false, false) => tlb.flush(),
            (true, false) => tlb.flush_vaddr(vpn()),
            (false, true) => tlb.flush_asid(asid()),
            (true, true) => tlb.flush_vaddr_asid(vpn(), asid()),
        }
    }
}

const fn mark_fp_dirty(hart: &mut Hart) {
    hart.csrs.mstatus = (hart.csrs.mstatus & !csr::MSTATUS_FS) | csr::MSTATUS_FS_DIRTY;
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

        assert!(pending_interrupt(state.hart).is_none());
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

        assert_eq!(pending_interrupt(state.hart), Some(Trap::MachineExternalInterrupt));
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

        assert_eq!(pending_interrupt(state.hart), Some(Trap::SupervisorExternalInterrupt));
    }
}
