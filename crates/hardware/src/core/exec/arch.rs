//! The architectural state an instruction reads as it executes.

use crate::common::CsrAddr;
use crate::core::Hart;

/// What executing an instruction may read, whichever engine executes it:
/// the hart, its CSRs as software sees them, and whether to trace.
pub trait ArchState {
    /// The hart executing the instruction.
    fn hart(&self) -> &Hart;

    /// A CSR's value, as a CSR instruction reads it.
    fn csr_read(&self, addr: CsrAddr) -> u64;

    /// The value a CSR read-modify-write starts from.
    fn csr_read_for_update(&self, addr: CsrAddr) -> u64;

    /// Whether instruction tracing is on.
    fn tracing(&self) -> bool;
}
