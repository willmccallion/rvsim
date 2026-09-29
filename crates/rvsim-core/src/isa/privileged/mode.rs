//! RISC-V Privilege Modes (User, Supervisor, Machine).

/// RISC-V privilege mode levels.
///
/// RISC-V defines three privilege modes that control access to system resources
/// and instructions. Machine mode is the highest privilege level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrivilegeMode {
    /// User mode (U-mode).
    ///
    /// Lowest privilege level for application code.
    User = 0,

    /// Supervisor mode (S-mode).
    ///
    /// Intermediate privilege level for operating system kernels.
    Supervisor = 1,

    /// Machine mode (M-mode).
    ///
    /// Highest privilege level for firmware and low-level system control.
    Machine = 3,
}

impl PrivilegeMode {
    /// Converts a `u8` value to a privilege mode. Defaults to `Machine` for invalid values.
    pub const fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::User,
            1 => Self::Supervisor,
            _ => Self::Machine,
        }
    }

    /// Converts a privilege mode to its `u8` representation.
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Returns the human-readable name of the privilege mode.
    pub const fn name(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Supervisor => "Supervisor",
            Self::Machine => "Machine",
        }
    }
}

impl std::fmt::Display for PrivilegeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}
