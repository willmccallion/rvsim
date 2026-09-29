//! Memory access type classification (Fetch / Read / Write).

/// Type of memory access operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessType {
    /// Instruction fetch access — requires X permission.
    Fetch,
    /// Data read access — requires R permission.
    Read,
    /// Data write access — requires W permission.
    Write,
}
