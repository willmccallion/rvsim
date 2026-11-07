//! The permission check a leaf page applies to an access.

use crate::common::AccessType;
use crate::isa::csr::{MSTATUS_MXR, MSTATUS_SUM};
use crate::isa::privileged::PrivilegeMode;

/// A leaf page's R, W, X and U bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct PagePerms {
    /// Readable.
    pub r: bool,
    /// Writable.
    pub w: bool,
    /// Executable.
    pub x: bool,
    /// User-mode accessible.
    pub u: bool,
}

impl PagePerms {
    /// Whether the page grants `access` at `privilege` under `mstatus`:
    /// MXR makes executable pages readable, and SUM lets supervisor mode
    /// read and write user pages, though never fetch from them.
    pub const fn permits(self, access: AccessType, privilege: PrivilegeMode, mstatus: u64) -> bool {
        let mxr = mstatus & MSTATUS_MXR != 0;
        let kind_allowed = match access {
            AccessType::Fetch => self.x,
            AccessType::Read => self.r || (self.x && mxr),
            AccessType::Write => self.w,
        };
        let privilege_allowed = match privilege {
            PrivilegeMode::User => self.u,
            PrivilegeMode::Supervisor if self.u => {
                mstatus & MSTATUS_SUM != 0 && !matches!(access, AccessType::Fetch)
            }
            PrivilegeMode::Supervisor | PrivilegeMode::Machine => true,
        };
        kind_allowed && privilege_allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: PagePerms = PagePerms { r: true, w: false, x: false, u: false };
    const RW: PagePerms = PagePerms { w: true, ..R };
    const X: PagePerms = PagePerms { r: false, w: false, x: true, u: false };
    const USER_RWX: PagePerms = PagePerms { r: true, w: true, x: true, u: true };
    const NONE: u64 = 0;

    #[test]
    fn each_access_needs_its_own_bit() {
        let s = PrivilegeMode::Supervisor;
        assert!(R.permits(AccessType::Read, s, NONE));
        assert!(!R.permits(AccessType::Write, s, NONE));
        assert!(!R.permits(AccessType::Fetch, s, NONE));
        assert!(RW.permits(AccessType::Write, s, NONE));
        assert!(X.permits(AccessType::Fetch, s, NONE));
    }

    #[test]
    fn mxr_makes_an_executable_page_readable() {
        let s = PrivilegeMode::Supervisor;
        assert!(!X.permits(AccessType::Read, s, NONE));
        assert!(X.permits(AccessType::Read, s, MSTATUS_MXR));
    }

    #[test]
    fn user_mode_reaches_only_user_pages() {
        assert!(!RW.permits(AccessType::Read, PrivilegeMode::User, NONE));
        assert!(USER_RWX.permits(AccessType::Write, PrivilegeMode::User, NONE));
    }

    #[test]
    fn sum_opens_user_pages_to_supervisor_loads_and_stores_but_not_fetches() {
        let s = PrivilegeMode::Supervisor;
        assert!(!USER_RWX.permits(AccessType::Read, s, NONE));
        assert!(USER_RWX.permits(AccessType::Read, s, MSTATUS_SUM));
        assert!(USER_RWX.permits(AccessType::Write, s, MSTATUS_SUM));
        assert!(!USER_RWX.permits(AccessType::Fetch, s, MSTATUS_SUM));
    }
}
