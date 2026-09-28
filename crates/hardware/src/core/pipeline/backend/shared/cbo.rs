//! Cache-block operations (Zicbom, Zicboz): the gate, and how each one
//! translates and faults.
//!
//! A CBO translates in memory1 like a load or store, and faults there to be
//! taken at commit; it then waits in the store buffer, ordered as a store,
//! and goes to the cache after it commits. `cbo.zero` needs write
//! permission. The management operations need only read
//! permission, but every CBO reports a fault as a store fault, as the CMO
//! specification (and Spike) does.

use crate::common::{AccessType, Trap};
use crate::core::arch::csr::{CboInvalAction, Csrs, cbo_inval_action, cbocf_allowed, cboz_allowed};
use crate::core::arch::mode::PrivilegeMode;
use crate::core::pipeline::signals::SystemOp;
use crate::isa::zicboz::CBOZ_BLOCK_SIZE;

/// What a CBO does to its block once its gate has passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CboEffect {
    /// Write zeros over the block.
    Zero,
    /// Write the block back if dirty and keep it.
    Clean,
    /// Write the block back if dirty and drop it.
    Flush,
    /// Drop the block without writing it back.
    Invalidate,
}

impl CboEffect {
    /// The permission the block's page must grant.
    #[must_use]
    pub const fn access(self) -> AccessType {
        match self {
            Self::Zero => AccessType::Write,
            Self::Clean | Self::Flush | Self::Invalidate => AccessType::Read,
        }
    }
}

/// The address a fault of CBO `op` on operand `rs1` reports: the block for
/// `cbo.zero`, whose first store faults; the operand for the others.
#[must_use]
pub const fn fault_address(op: SystemOp, rs1: u64) -> u64 {
    if matches!(op, SystemOp::CboZero) { block_address(rs1) } else { rs1 }
}

/// The block `addr` falls in.
#[must_use]
pub const fn block_address(addr: u64) -> u64 {
    addr & !(CBOZ_BLOCK_SIZE - 1)
}

/// Checks `op` against the `menvcfg`/`senvcfg` gates at `privilege` and
/// returns what it does, or the illegal-instruction trap `inst` raises.
///
/// # Errors
///
/// `Trap::IllegalInstruction` when the privilege level may not run `op`.
pub const fn gate(
    csrs: &Csrs,
    privilege: PrivilegeMode,
    op: SystemOp,
    inst: u32,
) -> Result<CboEffect, Trap> {
    let (menvcfg, senvcfg) = (csrs.menvcfg, csrs.senvcfg);
    let allowed = match op {
        SystemOp::CboZero if cboz_allowed(menvcfg, senvcfg, privilege) => Some(CboEffect::Zero),
        SystemOp::CboClean if cbocf_allowed(menvcfg, senvcfg, privilege) => Some(CboEffect::Clean),
        SystemOp::CboFlush if cbocf_allowed(menvcfg, senvcfg, privilege) => Some(CboEffect::Flush),
        SystemOp::CboInval => match cbo_inval_action(menvcfg, senvcfg, privilege) {
            CboInvalAction::Illegal => None,
            CboInvalAction::Flush => Some(CboEffect::Flush),
            CboInvalAction::Invalidate => Some(CboEffect::Invalidate),
        },
        _ => None,
    };
    match allowed {
        Some(effect) => Ok(effect),
        None => Err(Trap::IllegalInstruction(inst)),
    }
}

/// `trap`, raised translating a CBO, as the store fault the CBO reports at
/// `tval`.
#[must_use]
pub const fn as_store_fault(trap: Trap, tval: u64) -> Trap {
    match trap {
        Trap::LoadPageFault(_) | Trap::StorePageFault(_) => Trap::StorePageFault(tval),
        Trap::LoadAccessFault(_) | Trap::StoreAccessFault(_) => Trap::StoreAccessFault(tval),
        other => other,
    }
}
