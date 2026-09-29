//! System operations: fences, environment calls, trap returns, CSR access.

/// System operation classification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SystemOp {
    /// Not a system instruction.
    #[default]
    None,

    /// `MRET` — return from machine trap.
    Mret,

    /// `SRET` — return from supervisor trap.
    Sret,

    /// `WFI` — wait for interrupt.
    Wfi,

    /// `FENCE` — memory ordering fence.
    Fence,

    /// `FENCE.I` — instruction fence.
    FenceI,

    /// `SFENCE.VMA` — supervisor memory-management fence.
    SfenceVma,

    /// `CBO.ZERO` (Zicboz) — zero a cache-block-aligned region at rs1.
    CboZero,

    /// `CBO.INVAL` (Zicbom) — invalidate the L1D line at rs1.
    CboInval,

    /// `CBO.CLEAN` (Zicbom) — writeback the L1D line at rs1, keep it valid.
    CboClean,

    /// `CBO.FLUSH` (Zicbom) — writeback then invalidate the L1D line at rs1.
    CboFlush,

    /// `ECALL` — environment call.
    Ecall,

    /// `CSRRW`/`CSRRS`/`CSRRC` and their immediate forms; `csr_op` says which.
    Csr,
}

impl SystemOp {
    /// True for the instructions gem5 marks `IsSerializeAfter`: an
    /// out-of-order core renames nothing younger until they commit.
    pub const fn serializes_after(self) -> bool {
        matches!(
            self,
            Self::Csr
                | Self::Ecall
                | Self::Mret
                | Self::Sret
                | Self::Wfi
                | Self::SfenceVma
                | Self::FenceI
        )
    }

    /// True for the Zicboz/Zicbom cache-block operations.
    pub const fn is_cbo(self) -> bool {
        matches!(self, Self::CboZero | Self::CboInval | Self::CboClean | Self::CboFlush)
    }
}

/// CSR (Control and Status Register) operation type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CsrOp {
    /// No CSR operation.
    #[default]
    None,

    /// CSR read-write (`CSRRW`).
    Rw,

    /// CSR read-set (`CSRRS`).
    Rs,

    /// CSR read-clear (`CSRRC`).
    Rc,

    /// CSR read-write immediate (`CSRRWI`).
    Rwi,

    /// CSR read-set immediate (`CSRRSI`).
    Rsi,

    /// CSR read-clear immediate (`CSRRCI`).
    Rci,
}
