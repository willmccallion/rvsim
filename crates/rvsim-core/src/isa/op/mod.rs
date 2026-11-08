//! The operations instructions perform, as the decoder names them.

mod alu;
mod format;
mod memory;
pub mod pseudo;
mod system;
mod vector;

pub use alu::{
    AluOp, AluOpGroup, ArithOp, BitOp, ClmulOp, CountOp, DivOp, FmaSigns, FpArithOp, FpCompareOp,
    FpOp, IntAluOp, LogicOp, MinMaxOp, MulOp, PackOp, RotateOp, ShiftAddOp, ShiftOp, SignInjectOp,
    SingleBitOp,
};
pub use format::{IntWidth, OperandFormat, Signedness};
pub use memory::{AmoOp, AtomicOp, MemWidth};
pub use system::{CsrOp, SystemOp};
pub use vector::{
    CarryOp, CompareOp, CryptoOp, ExtendFactor, ExtendOp, FmaAddend, FpBinaryOp, FpNarrowConvertOp,
    FpReduceOp, FpUnaryOp, FpWidenConvertOp, FpWidenOp, FpWidenReduceOp, IntConvert, IntOp,
    IntReduceOp, IntRounding, MaccOp, MaskLogicalOp, MaskOp, MaskSetOp, NarrowOp, PermuteOp,
    ReduceOp, SlideOffset, VecAluOp, VecConfigOp, VecFpOp, VecLoadOp, VecOpGroup, VecOperandGroups,
    VecSrcEncoding, VecStoreOp, VectorOp, WholeRegisters, WidenIntReduceOp, WidenMaccOp, WidenOp,
};
