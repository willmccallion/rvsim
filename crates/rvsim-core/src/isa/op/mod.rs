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
    CarryOp, CompareOp, CryptoOp, ExtendOp, FmaAddend, FmaSigns, FpBinaryOp, FpCompareOp,
    FpNarrowConvertOp, FpReduceOp, FpUnaryOp, FpWidenConvertOp, FpWidenOp, FpWidenReduceOp,
    IntConvert, IntOp, IntReduceOp, IntRounding, MaccOp, MaskLogicalOp, MaskOp, MaskSetOp,
    NarrowOp, PermuteOp, ReduceOp, Signedness, SlideOffset, VecAluOp, VecClass, VecFpOp,
    VecOpGroup, VecOperandGroups, VecSrcEncoding, VectorOp, WidenIntReduceOp, WidenMaccOp, WidenOp,
};
