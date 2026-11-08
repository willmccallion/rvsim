//! The ALU and FPU operations an instruction decodes to.

use super::format::{IntWidth, Signedness};
use crate::isa::fp::FpFormat;

/// The operation an integer or floating-point instruction performs, by
/// the unit that computes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AluOp {
    /// The integer ALU.
    Int(IntAluOp),
    /// The floating-point unit.
    Fp(FpOp),
}

impl Default for AluOp {
    /// An add, which address generation and the no-op use.
    fn default() -> Self {
        Self::Int(IntAluOp::Arith(ArithOp::Add))
    }
}

/// The operations the integer ALU performs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntAluOp {
    /// Addition and subtraction.
    Arith(ArithOp),
    /// Bitwise logic and set-less-than.
    Logic(LogicOp),
    /// Shifts.
    Shift(ShiftOp),
    /// M-extension multiplication.
    Mul(MulOp),
    /// M-extension division and remainder.
    Div(DivOp),
    /// Bit manipulation (Zba, Zbb, Zbc, Zbs, Zbkb, Zbkx).
    Bit(BitOp),
}

/// The `From` impls that lift each group of integer operations into an
/// [`IntAluOp`] and an [`AluOp`].
macro_rules! into_alu_op {
    ($($group:ident => $variant:ident),*) => {$(
        impl From<$group> for IntAluOp {
            fn from(op: $group) -> Self {
                Self::$variant(op)
            }
        }

        impl From<$group> for AluOp {
            fn from(op: $group) -> Self {
                Self::Int(IntAluOp::$variant(op))
            }
        }
    )*};
}

into_alu_op!(
    ArithOp => Arith, LogicOp => Logic, ShiftOp => Shift, MulOp => Mul, DivOp => Div, BitOp => Bit
);

impl From<IntAluOp> for AluOp {
    fn from(op: IntAluOp) -> Self {
        Self::Int(op)
    }
}

impl From<FpOp> for AluOp {
    fn from(op: FpOp) -> Self {
        Self::Fp(op)
    }
}

/// Addition and subtraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOp {
    /// `add`.
    Add,
    /// `sub`.
    Sub,
}

/// Bitwise logic and set-less-than.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicOp {
    /// `and`.
    And,
    /// `or`.
    Or,
    /// `xor`.
    Xor,
    /// `slt`: signed set-less-than.
    Slt,
    /// `sltu`: unsigned set-less-than.
    Sltu,
}

/// Shifts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftOp {
    /// `sll`.
    Sll,
    /// `srl`.
    Srl,
    /// `sra`.
    Sra,
}

/// M-extension multiplication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MulOp {
    /// `mul`: the low half of the product.
    Mul,
    /// `mulh`: the high half, both operands signed.
    Mulh,
    /// `mulhsu`: the high half, `rs1` signed and `rs2` unsigned.
    Mulhsu,
    /// `mulhu`: the high half, both unsigned.
    Mulhu,
}

/// M-extension division and remainder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DivOp {
    /// `div`.
    Div,
    /// `divu`.
    Divu,
    /// `rem`.
    Rem,
    /// `remu`.
    Remu,
}

/// Bit manipulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitOp {
    /// Zba address generation.
    ShiftAdd(ShiftAddOp),
    /// `andn`: `a & !b`.
    Andn,
    /// `orn`: `a | !b`.
    Orn,
    /// `xnor`: `!(a ^ b)`.
    Xnor,
    /// Bit counts.
    Count(CountOp),
    /// Integer minimum and maximum.
    MinMax(MinMaxOp),
    /// `sext.b`.
    SextB,
    /// `sext.h`.
    SextH,
    /// Rotates.
    Rotate(RotateOp),
    /// `orc.b`: each byte all ones if any of its bits is set.
    OrcB,
    /// `rev8`: the bytes reversed.
    Rev8,
    /// `brev8`: the bits of each byte reversed.
    Brev8,
    /// Carry-less multiplication.
    Clmul(ClmulOp),
    /// Single-bit operations (Zbs).
    SingleBit(SingleBitOp),
    /// Packing (Zbkb).
    Pack(PackOp),
    /// `xperm4`: a crossbar permute of nibbles.
    Xperm4,
    /// `xperm8`: a crossbar permute of bytes.
    Xperm8,
}

/// Zba address generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftAddOp {
    /// `sh1add`.
    Sh1Add,
    /// `sh2add`.
    Sh2Add,
    /// `sh3add`.
    Sh3Add,
    /// `add.uw`.
    AddUw,
    /// `sh1add.uw`.
    Sh1AddUw,
    /// `sh2add.uw`.
    Sh2AddUw,
    /// `sh3add.uw`.
    Sh3AddUw,
    /// `slli.uw`.
    SlliUw,
}

/// Bit counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountOp {
    /// `clz`: leading zeros.
    Clz,
    /// `ctz`: trailing zeros.
    Ctz,
    /// `cpop`: set bits.
    Cpop,
}

/// Integer minimum and maximum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinMaxOp {
    /// `max`.
    Max,
    /// `maxu`.
    Maxu,
    /// `min`.
    Min,
    /// `minu`.
    Minu,
}

/// Rotates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotateOp {
    /// `rol`.
    Rol,
    /// `ror`.
    Ror,
}

/// Carry-less multiplication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClmulOp {
    /// `clmul`: the low half of the product.
    Clmul,
    /// `clmulh`: the high half.
    Clmulh,
    /// `clmulr`: bits 126 to 63.
    Clmulr,
}

/// Single-bit operations on the bit of `rs1` that `rs2` numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SingleBitOp {
    /// `bclr`.
    Bclr,
    /// `bext`.
    Bext,
    /// `binv`.
    Binv,
    /// `bset`.
    Bset,
}

/// Packing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackOp {
    /// `pack`: the low halves of `rs1` and `rs2`.
    Pack,
    /// `packh`: the low bytes.
    Packh,
    /// `packw`: the low 16 bits of each, sign-extended from a word.
    Packw,
}

/// Floating-point operations. The format is the instruction's
/// [`OperandFormat`](super::OperandFormat).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpOp {
    /// Rounded binary arithmetic.
    Arith(FpArithOp),
    /// `fsqrt`.
    Sqrt,
    /// A fused multiply-add of `rs1 × rs2` and `rs3`.
    Fma(FmaSigns),
    /// `fmin` (IEEE 754-2008 minNum).
    Min,
    /// `fmax` (maxNum).
    Max,
    /// Sign injection.
    SignInject(SignInjectOp),
    /// A comparison into an integer register.
    Compare(FpCompareOp),
    /// `fclass`.
    Class,
    /// `fmv.x.*`: the bits into an integer register.
    MoveToInt,
    /// `fmv.*.x`: the bits from an integer register.
    MoveFromInt,
    /// `fcvt` to an integer.
    ToInt(IntWidth, Signedness),
    /// `fcvt` from an integer.
    FromInt(IntWidth, Signedness),
    /// `fcvt` from the floating-point format given.
    FromFloat(FpFormat),
}

/// Rounded binary floating-point arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpArithOp {
    /// `fadd`.
    Add,
    /// `fsub`.
    Sub,
    /// `fmul`.
    Mul,
    /// `fdiv`.
    Div,
}

/// The signs of a fused multiply-add's product and addend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FmaSigns {
    /// `+(a × b) + c`: `fmadd`, `vfmacc`.
    Macc,
    /// `-(a × b) - c`: `fnmadd`, `vfnmacc`.
    NMacc,
    /// `+(a × b) - c`: `fmsub`, `vfmsac`.
    MSac,
    /// `-(a × b) + c`: `fnmsub`, `vfnmsac`.
    NMSac,
}

/// Sign injection: the magnitude of `rs1` with a sign from `rs2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInjectOp {
    /// `fsgnj`: `rs2`'s sign.
    Copy,
    /// `fsgnjn`: its opposite.
    Negate,
    /// `fsgnjx`: the two signs' exclusive or.
    Xor,
}

/// Floating-point comparisons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpCompareOp {
    /// Equal (`feq`, `vmfeq`).
    Eq,
    /// Not equal (`vmfne`).
    Ne,
    /// Less than (`flt`, `vmflt`).
    Lt,
    /// Less than or equal (`fle`, `vmfle`).
    Le,
    /// Greater than (`vmfgt`).
    Gt,
    /// Greater than or equal (`vmfge`).
    Ge,
}

/// Which kind of ALU or FPU operation this is, finely enough that both the
/// unit it runs on and the commit statistic that counts it follow from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AluOpGroup {
    /// Integer arithmetic, logic, shifts and bit manipulation.
    IntAlu,
    /// Integer multiplication.
    IntMul,
    /// Integer division and remainder.
    IntDiv,
    /// Floating-point arithmetic, comparison, conversion and moves, but for the groups below.
    FpArith,
    /// Floating-point multiplication.
    FpMul,
    /// Floating-point division and square root.
    FpDivSqrt,
    /// Floating-point fused multiply-add.
    FpFma,
}

impl AluOp {
    /// The group this operation belongs to.
    #[must_use]
    pub const fn group(self) -> AluOpGroup {
        match self {
            Self::Int(IntAluOp::Mul(_)) => AluOpGroup::IntMul,
            Self::Int(IntAluOp::Div(_)) => AluOpGroup::IntDiv,
            Self::Int(_) => AluOpGroup::IntAlu,
            Self::Fp(FpOp::Arith(FpArithOp::Mul)) => AluOpGroup::FpMul,
            Self::Fp(FpOp::Arith(FpArithOp::Div) | FpOp::Sqrt) => AluOpGroup::FpDivSqrt,
            Self::Fp(FpOp::Fma(_)) => AluOpGroup::FpFma,
            Self::Fp(_) => AluOpGroup::FpArith,
        }
    }
}
