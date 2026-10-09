//! Sdtrig debug triggers: which addresses match an armed `mcontrol` trigger.
//!
//! Each trigger is an `mcontrol` (type 2) address match that raises a
//! breakpoint exception: it matches on equality, never chains and always
//! fires before the access, so those fields read as zero.

use crate::arch::Hart;
use crate::isa::privileged::PrivilegeMode;

/// The triggers the hart implements.
pub const TRIGGER_COUNT: usize = 2;

/// `tdata1.type` of an address match trigger.
const MCONTROL_TYPE: u64 = 2;
const TYPE_SHIFT: u32 = 60;

const MCONTROL_LOAD: u64 = 1 << 0;
const MCONTROL_STORE: u64 = 1 << 1;
const MCONTROL_EXECUTE: u64 = 1 << 2;
const MCONTROL_U: u64 = 1 << 3;
const MCONTROL_S: u64 = 1 << 4;
const MCONTROL_M: u64 = 1 << 6;

/// `tcontrol.mte`: triggers fire in M-mode only while it is set.
const TCONTROL_MTE: u64 = 1 << 3;

/// The value `tdata1` takes when `val` is written to it: an `mcontrol`
/// keeps its type, privilege modes and access kinds; any other type turns
/// the trigger off.
#[must_use]
pub const fn tdata1_written(val: u64) -> u64 {
    const WRITABLE: u64 = (0xF << TYPE_SHIFT)
        | MCONTROL_M
        | MCONTROL_S
        | MCONTROL_U
        | MCONTROL_EXECUTE
        | MCONTROL_STORE
        | MCONTROL_LOAD;
    if val >> TYPE_SHIFT == MCONTROL_TYPE { val & WRITABLE } else { 0 }
}

impl Hart {
    /// Returns true if an execute trigger fires for the given PC and current privilege.
    #[must_use]
    pub fn check_execute_trigger(&self, pc: u64) -> bool {
        self.trigger_fires(MCONTROL_EXECUTE, pc)
    }

    /// Returns true if a load trigger fires for the given address and current privilege.
    #[must_use]
    pub fn check_load_trigger(&self, addr: u64) -> bool {
        self.trigger_fires(MCONTROL_LOAD, addr)
    }

    /// Returns true if a store trigger fires for the given address and current privilege.
    #[must_use]
    pub fn check_store_trigger(&self, addr: u64) -> bool {
        self.trigger_fires(MCONTROL_STORE, addr)
    }

    /// True when an `mcontrol` trigger armed for `access` in the current
    /// privilege mode matches `addr`.
    fn trigger_fires(&self, access: u64, addr: u64) -> bool {
        (0..TRIGGER_COUNT).any(|i| {
            let tdata1 = self.csrs.tdata1[i];
            tdata1 >> TYPE_SHIFT == MCONTROL_TYPE
                && tdata1 & access != 0
                && self.trigger_enabled_in_current_mode(tdata1)
                && self.csrs.tdata2[i] == addr
        })
    }

    const fn trigger_enabled_in_current_mode(&self, tdata1: u64) -> bool {
        match self.privilege {
            PrivilegeMode::Machine => {
                tdata1 & MCONTROL_M != 0 && self.csrs.tcontrol & TCONTROL_MTE != 0
            }
            PrivilegeMode::Supervisor => tdata1 & MCONTROL_S != 0,
            PrivilegeMode::User => tdata1 & MCONTROL_U != 0,
        }
    }
}
