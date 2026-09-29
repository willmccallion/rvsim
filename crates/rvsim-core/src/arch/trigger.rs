//! Sdtrig debug triggers: which addresses match an armed `mcontrol6` trigger.

use crate::arch::Hart;
use crate::isa::privileged::PrivilegeMode;

impl Hart {
    /// Returns true if an execute trigger fires for the given PC and current privilege.
    pub fn check_execute_trigger(&self, pc: u64) -> bool {
        let mte = (self.csrs.tcontrol >> 3) & 1 != 0;
        for i in 0..2usize {
            let tdata1 = self.csrs.tdata1[i];
            if (tdata1 >> 60) & 0xF != 2 {
                continue;
            } // not mcontrol
            if (tdata1 >> 9) & 1 == 0 {
                continue;
            } // not execute trigger
            let action = (tdata1 >> 19) & 0x3;
            if action != 0 {
                continue;
            } // only breakpoint exception
            let mode_ok = match self.privilege {
                PrivilegeMode::Machine => (tdata1 >> 13) & 1 != 0 && mte,
                PrivilegeMode::Supervisor => (tdata1 >> 11) & 1 != 0,
                PrivilegeMode::User => (tdata1 >> 10) & 1 != 0,
            };
            if mode_ok && self.csrs.tdata2[i] == pc {
                return true;
            }
        }
        false
    }

    /// Returns true if a load trigger fires for the given address and current privilege.
    pub fn check_load_trigger(&self, addr: u64) -> bool {
        let mte = (self.csrs.tcontrol >> 3) & 1 != 0;
        for i in 0..2usize {
            let tdata1 = self.csrs.tdata1[i];
            if (tdata1 >> 60) & 0xF != 2 {
                continue;
            }
            if (tdata1 >> 7) & 1 == 0 {
                continue;
            } // not load trigger
            let action = (tdata1 >> 19) & 0x3;
            if action != 0 {
                continue;
            }
            let mode_ok = match self.privilege {
                PrivilegeMode::Machine => (tdata1 >> 13) & 1 != 0 && mte,
                PrivilegeMode::Supervisor => (tdata1 >> 11) & 1 != 0,
                PrivilegeMode::User => (tdata1 >> 10) & 1 != 0,
            };
            if mode_ok && self.csrs.tdata2[i] == addr {
                return true;
            }
        }
        false
    }

    /// Returns true if a store trigger fires for the given address and current privilege.
    pub fn check_store_trigger(&self, addr: u64) -> bool {
        let mte = (self.csrs.tcontrol >> 3) & 1 != 0;
        for i in 0..2usize {
            let tdata1 = self.csrs.tdata1[i];
            if (tdata1 >> 60) & 0xF != 2 {
                continue;
            }
            if (tdata1 >> 8) & 1 == 0 {
                continue;
            } // not store trigger
            let action = (tdata1 >> 19) & 0x3;
            if action != 0 {
                continue;
            }
            let mode_ok = match self.privilege {
                PrivilegeMode::Machine => (tdata1 >> 13) & 1 != 0 && mte,
                PrivilegeMode::Supervisor => (tdata1 >> 11) & 1 != 0,
                PrivilegeMode::User => (tdata1 >> 10) & 1 != 0,
            };
            if mode_ok && self.csrs.tdata2[i] == addr {
                return true;
            }
        }
        false
    }
}
