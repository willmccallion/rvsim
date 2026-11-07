//! The operations instructions perform, as the decoder names them.

mod alu;
mod format;
mod memory;
pub mod pseudo;
mod system;
mod vector;

pub use alu::{AluOp, AluOpGroup};
pub use format::{IntWidth, OperandFormat};
pub use memory::{AtomicOp, MemWidth};
pub use system::{CsrOp, SystemOp};
pub use vector::{
    CarryOp, CompareOp, CryptoOp, ExtendOp, FpReduceOp, FpWidenReduceOp, IntOp, IntReduceOp,
    MaccOp, MaskLogicalOp, MaskOp, MaskSetOp, NarrowOp, PermuteOp, ReduceOp, SlideOffset, VecAluOp,
    VecClass, VecOpGroup, VecOperandGroups, VecSrcEncoding, VectorOp, WidenIntReduceOp,
    WidenMaccOp, WidenOp,
};
