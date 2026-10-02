//! The operations instructions perform, as the decoder names them.

mod alu;
mod memory;
mod system;
mod vector;

pub use alu::AluOp;
pub use memory::{AtomicOp, MemWidth};
pub use system::{CsrOp, SystemOp};
pub use vector::{
    CarryOp, CompareOp, CryptoOp, ExtendOp, FpReduceOp, FpWidenReduceOp, IntOp, IntReduceOp,
    MaccOp, MaskLogicalOp, MaskOp, MaskSetOp, NarrowOp, PermuteOp, ReduceOp, SlideOffset, VecAluOp,
    VecClass, VecOperandGroups, VecSrcEncoding, VectorOp, WidenIntReduceOp, WidenMaccOp, WidenOp,
};
