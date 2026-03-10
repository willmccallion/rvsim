//! CSR address newtype.
//!
//! RISC-V CSR addresses are 12-bit values (0x000–0xFFF) encoded in
//! bits 31:20 of I-type instructions. This module provides [`CsrAddr`],
//! a strong newtype that documents this constraint and prevents a raw
//! `u32` from being accidentally used as a CSR address.
//!
//! All CSR address constants in `crate::core::arch::csr` are `CsrAddr`,
//! and `csr_read` / `csr_write` accept `CsrAddr` directly.

/// A 12-bit CSR (Control and Status Register) address (0x000–0xFFF).
///
/// CSR addresses are encoded as a 12-bit immediate in the instruction
/// word (bits 31:20). Bits above 12 are always zero.
///
/// # Example
///
/// ```ignore
/// use rvsim_core::core::arch::csr;
///
/// let val = cpu.csr_read(csr::SATP);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct CsrAddr(u16);

impl CsrAddr {
    /// Creates a `CsrAddr` from a raw `u16`, masking to 12 bits.
    ///
    /// Values above `0xFFF` are truncated.
    #[inline(always)]
    pub const fn new(addr: u16) -> Self {
        Self(addr & 0xFFF)
    }

    /// Creates a `CsrAddr` from a `u32` CSR constant.
    ///
    /// This is the primary conversion when working with the existing
    /// `u32` CSR address constants and the output of `InstructionBits::csr()`.
    ///
    /// Values above `0xFFF` are truncated (the high bits are always
    /// zero for valid CSR addresses).
    #[inline(always)]
    pub const fn from_u32(addr: u32) -> Self {
        Self((addr & 0xFFF) as u16)
    }

    /// Returns the address as a `u32` for use in match arms and PMP range checks.
    #[inline(always)]
    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }

    /// Returns the raw `u16` value.
    #[inline(always)]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Extracts the privilege level encoded in bits 9:8 of the CSR address.
    ///
    /// Per the RISC-V privileged spec (§2.1):
    /// - `0b00` = Unprivileged / User
    /// - `0b01` = Supervisor
    /// - `0b10` = Hypervisor
    /// - `0b11` = Machine
    #[inline(always)]
    pub const fn privilege_level(self) -> u8 {
        ((self.0 >> 8) & 0x3) as u8
    }

    /// Returns `true` if the CSR is read-only.
    ///
    /// Per the RISC-V privileged spec (§2.1): a CSR is read-only when
    /// bits 11:10 of its address are both `1` (`0b11`).
    #[inline(always)]
    pub const fn is_read_only(self) -> bool {
        ((self.0 >> 10) & 0x3) == 0x3
    }
}

impl From<u16> for CsrAddr {
    #[inline(always)]
    fn from(v: u16) -> Self {
        Self::new(v)
    }
}

impl From<CsrAddr> for u32 {
    #[inline(always)]
    fn from(c: CsrAddr) -> Self {
        c.0 as Self
    }
}

impl std::fmt::Display for CsrAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CSR({:#05x})", self.0)
    }
}

impl std::fmt::LowerHex for CsrAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::LowerHex::fmt(&self.0, f)
    }
}
