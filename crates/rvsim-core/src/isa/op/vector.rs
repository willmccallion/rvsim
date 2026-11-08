//! The vector operations an instruction decodes to.

use super::alu::{FmaSigns, FpCompareOp};
use super::format::Signedness;

/// Which kind of vector operation this is, finely enough that both the
/// unit it runs on and the commit statistic that counts it follow from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecOpGroup {
    /// `vsetvl` and its immediate forms.
    Config,
    /// A load.
    Load,
    /// A store.
    Store,
    /// Integer arithmetic, logic, comparison and reduction.
    IntAlu,
    /// Integer multiplication and multiply-add.
    IntMul,
    /// Integer division and remainder.
    IntDiv,
    /// A vector cryptography instruction, which runs on the integer ALU.
    Crypto,
    /// Floating-point arithmetic other than multiplication, division and square root.
    FpAlu,
    /// Floating-point multiplication and multiply-add.
    FpFma,
    /// Floating-point division and square root.
    FpDivSqrt,
    /// A floating-point slide, which runs on the permute unit.
    FpSlide,
    /// Mask logic, gathers, slides, compress and whole-register moves.
    Permute,
}

/// A vector operation, by the unit and group of operations it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorOp {
    /// `vsetvl` and its immediate forms.
    Config(VecConfigOp),
    /// A load.
    Load(VecLoadOp),
    /// A store.
    Store(VecStoreOp),
    /// The integer ALU.
    Alu(VecAluOp),
    /// A floating-point operation.
    Fp(VecFpOp),
    /// A reduction.
    Reduce(ReduceOp),
    /// A mask operation.
    Mask(MaskOp),
    /// A permutation.
    Permute(PermuteOp),
    /// A cryptography operation.
    Crypto(CryptoOp),
}

/// The configuration instructions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecConfigOp {
    /// `vsetvli`: the AVL from rs1, the vtype immediate.
    Vsetvli {
        /// The requested `vtype`.
        vtype: u16,
    },
    /// `vsetivli`: both immediate.
    Vsetivli {
        /// The application vector length.
        avl: u8,
        /// The requested `vtype`.
        vtype: u16,
    },
    /// `vsetvl`: the AVL from rs1, vtype from rs2.
    Vsetvl,
}

/// How a vector load addresses its elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecLoadOp {
    /// `vle`: unit stride, and the segment forms.
    Unit,
    /// `vleff`: unit stride, trapping only on element 0.
    FaultOnlyFirst,
    /// `vlm`: a mask register's bytes.
    Mask,
    /// `vl<n>re`: whole registers.
    WholeRegister,
    /// `vlse`: strided.
    Strided,
    /// `vloxei`: indexed, ordered.
    IndexedOrdered,
    /// `vluxei`: indexed, unordered.
    IndexedUnordered,
}

/// How a vector store addresses its elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecStoreOp {
    /// `vse`: unit stride, and the segment forms.
    Unit,
    /// `vsm`: a mask register's bytes.
    Mask,
    /// `vs<n>r`: whole registers.
    WholeRegister,
    /// `vsse`: strided.
    Strided,
    /// `vsoxei`: indexed, ordered.
    IndexedOrdered,
    /// `vsuxei`: indexed, unordered.
    IndexedUnordered,
}

/// Per-operand vector register group sizes for a given instruction.
///
/// Models how real hardware derives operand grouping from the opcode and LMUL.
/// A value of 0 means the operand field is not a vector register (it may be a
/// scalar GPR/FPR or a sub-opcode selector encoded in the vs1/vs2 field).
#[derive(Clone, Copy, Debug)]
pub struct VecOperandGroups {
    /// Number of registers in vd group (0 = scalar/sub-opcode, not a vreg).
    pub vd: u8,
    /// Number of registers in vs1 group (0 = scalar/immediate/sub-opcode).
    pub vs1: u8,
    /// Number of registers in vs2 group (0 = not used as vreg source).
    pub vs2: u8,
}

impl VecOperandGroups {
    /// No vector register operands.
    pub const NONE: Self = Self { vd: 0, vs1: 0, vs2: 0 };
}

impl VectorOp {
    /// The group this operation belongs to.
    #[must_use]
    pub const fn group(self) -> VecOpGroup {
        match self {
            Self::Config(_) => VecOpGroup::Config,
            Self::Load(_) => VecOpGroup::Load,
            Self::Store(_) => VecOpGroup::Store,
            Self::Alu(
                VecAluOp::Int(
                    IntOp::Add
                    | IntOp::Sub
                    | IntOp::Rsub
                    | IntOp::And
                    | IntOp::Or
                    | IntOp::Xor
                    | IntOp::Sll
                    | IntOp::Srl
                    | IntOp::Sra
                    | IntOp::MinU
                    | IntOp::Min
                    | IntOp::MaxU
                    | IntOp::Max
                    | IntOp::SAddU
                    | IntOp::SAdd
                    | IntOp::SSubU
                    | IntOp::SSub
                    | IntOp::AAddU
                    | IntOp::AAdd
                    | IntOp::ASubU
                    | IntOp::ASub
                    | IntOp::Smul
                    | IntOp::SSrl
                    | IntOp::SSra
                    | IntOp::AndN
                    | IntOp::Brev
                    | IntOp::Brev8
                    | IntOp::Rev8
                    | IntOp::Clz
                    | IntOp::Ctz
                    | IntOp::CpopV
                    | IntOp::Rol
                    | IntOp::Ror,
                )
                | VecAluOp::Merge
                | VecAluOp::Compare(_)
                | VecAluOp::Carry(_)
                | VecAluOp::Widen(
                    WidenOp::AddU
                    | WidenOp::Add
                    | WidenOp::SubU
                    | WidenOp::Sub
                    | WidenOp::AddUW
                    | WidenOp::AddW
                    | WidenOp::SubUW
                    | WidenOp::SubW
                    | WidenOp::Sll,
                )
                | VecAluOp::Narrow(_)
                | VecAluOp::Extend(_),
            )
            | Self::Reduce(ReduceOp::Int(_) | ReduceOp::WidenInt(_)) => VecOpGroup::IntAlu,
            Self::Alu(
                VecAluOp::Int(
                    IntOp::Mul
                    | IntOp::Mulh
                    | IntOp::Mulhu
                    | IntOp::Mulhsu
                    | IntOp::ClMul
                    | IntOp::ClMulH,
                )
                | VecAluOp::Macc(_)
                | VecAluOp::Widen(WidenOp::MulU | WidenOp::Mul | WidenOp::MulSU)
                | VecAluOp::WidenMacc(_),
            ) => VecOpGroup::IntMul,
            Self::Alu(VecAluOp::Int(IntOp::DivU | IntOp::Div | IntOp::RemU | IntOp::Rem)) => {
                VecOpGroup::IntDiv
            }
            Self::Crypto(_) => VecOpGroup::Crypto,
            Self::Fp(
                VecFpOp::Binary(
                    FpBinaryOp::Add
                    | FpBinaryOp::Sub
                    | FpBinaryOp::RSub
                    | FpBinaryOp::Min
                    | FpBinaryOp::Max
                    | FpBinaryOp::Sgnj
                    | FpBinaryOp::Sgnjn
                    | FpBinaryOp::Sgnjx,
                )
                | VecFpOp::Compare(_)
                | VecFpOp::Unary(
                    FpUnaryOp::Rsqrt7 | FpUnaryOp::Rec7 | FpUnaryOp::Class | FpUnaryOp::Convert(_),
                )
                | VecFpOp::Widen(FpWidenOp::Add | FpWidenOp::Sub | FpWidenOp::AddW | FpWidenOp::SubW)
                | VecFpOp::WidenConvert(_)
                | VecFpOp::NarrowConvert(_)
                | VecFpOp::Merge
                | VecFpOp::MoveFromScalar
                | VecFpOp::MoveToScalar,
            )
            | Self::Reduce(ReduceOp::Fp(_) | ReduceOp::FpWiden(_)) => VecOpGroup::FpAlu,
            Self::Fp(
                VecFpOp::Binary(FpBinaryOp::Mul)
                | VecFpOp::Fma(_, _)
                | VecFpOp::Widen(FpWidenOp::Mul)
                | VecFpOp::WidenFma(_),
            ) => VecOpGroup::FpFma,
            Self::Fp(
                VecFpOp::Binary(FpBinaryOp::Div | FpBinaryOp::RDiv)
                | VecFpOp::Unary(FpUnaryOp::Sqrt),
            ) => VecOpGroup::FpDivSqrt,
            Self::Fp(VecFpOp::Slide1Up | VecFpOp::Slide1Down) => VecOpGroup::FpSlide,
            Self::Mask(_) | Self::Permute(_) => VecOpGroup::Permute,
        }
    }

    /// `vsetvli`, `vsetivli` or `vsetvl`: writes `vtype` and `vl` instead of
    /// a vector register.
    #[must_use]
    pub const fn is_config(self) -> bool {
        matches!(self, Self::Config(_))
    }

    /// True for every vector op `vtype` governs, which therefore traps
    /// while `vtype.vill` is set (RVV 1.0 §3.4.4): all but the
    /// configuration ops and the whole-register loads, stores and moves.
    #[must_use]
    pub const fn depends_on_vtype(self) -> bool {
        !matches!(
            self,
            Self::Config(_)
                | Self::Load(VecLoadOp::WholeRegister)
                | Self::Store(VecStoreOp::WholeRegister)
                | Self::Permute(PermuteOp::WholeMove(_))
        )
    }

    /// True for the ops that execute even when `vstart` ≥ `vl`: those that
    /// write an `x` or `f` register, and the whole-register moves, which
    /// ignore `vl`. Every other op then updates no destination element.
    #[must_use]
    pub const fn executes_without_body(self) -> bool {
        matches!(
            self,
            Self::Permute(PermuteOp::MvXS | PermuteOp::WholeMove(_))
                | Self::Fp(VecFpOp::MoveToScalar)
                | Self::Mask(MaskOp::CPop | MaskOp::First)
        )
    }

    /// Compute the vector register group size for each operand given the base
    /// LMUL (1, 2, 4, or 8) and the source encoding.
    ///
    /// This is the single source of truth for operand grouping — every pipeline
    /// stage (decode alignment check, rename, execute sync, commit) should call
    /// this rather than maintaining separate per-stage logic.
    ///
    /// RVV 1.0 operand semantics:
    ///  - Most arithmetic: vd, vs2, vs1 are all LMUL-sized groups.
    ///  - Widening: vd = 2×LMUL, vs2 = LMUL (or 2×LMUL for .wv/.wf), vs1 = LMUL.
    ///  - Narrowing: vd = LMUL, vs2 = 2×LMUL.
    ///  - Scalar-result (vmv.x.s, vcpop, vfirst, vfmv.f.s): vd = 0 (scalar GPR/FPR).
    ///  - Mask-destination (comparisons, vmadc, vmsbf …): vd = 1 (single mask reg).
    ///  - Mask-source (vcpop, vmsbf …): vs2 = 1 (single mask reg).
    ///  - Sub-opcode in vs1 (UNARY0 families): vs1 = 0.
    ///  - Mask logical: all operands are single mask registers = 1.
    ///  - Whole-register moves/loads/stores: fixed group sizes independent of LMUL.
    ///
    /// `lmul` is the source register-group count (1/2/4/8 for both fractional
    /// LMUL and M1 — they all share group=1). `lmul_is_fractional` distinguishes
    /// fractional LMUL (Mf8/Mf4/Mf2) from M1: it matters for widening, where
    /// `2*LMUL` for fractional still fits in 1 register but `2*M1 = M2` needs 2.
    pub fn operand_groups(
        self,
        lmul: u8,
        lmul_is_fractional: bool,
        src_enc: VecSrcEncoding,
        nf: u8,
        broadcast_vs2: bool,
    ) -> VecOperandGroups {
        // Doubled register group for widening / narrowing. For fractional LMUL,
        // `2*LMUL` is still ≤ 1 register (e.g. 2*Mf8 = Mf4), so emul_widened = 1
        // even though `lmul == 1` would otherwise produce 2.
        let emul_widened: u8 = if lmul_is_fractional { 1 } else { (lmul * 2).min(8) };

        // vs1 is only a vector register for VV encoding; for VX/VI/VF it's a
        // scalar or immediate, so group size = 0.
        let vs1_base = if src_enc == VecSrcEncoding::VV { lmul } else { 0 };

        // .vs-form crypto ops broadcast vs2 element group 0; vs2 EMUL is 1.
        let vs2_crypto = if broadcast_vs2 { 1 } else { lmul };

        match self {
            Self::Config(_) => VecOperandGroups::NONE,
            Self::Alu(
                VecAluOp::Int(
                    IntOp::Add
                    | IntOp::Sub
                    | IntOp::Rsub
                    | IntOp::And
                    | IntOp::Or
                    | IntOp::Xor
                    | IntOp::Sll
                    | IntOp::Srl
                    | IntOp::Sra
                    | IntOp::MinU
                    | IntOp::Min
                    | IntOp::MaxU
                    | IntOp::Max
                    | IntOp::Mul
                    | IntOp::Mulh
                    | IntOp::Mulhu
                    | IntOp::Mulhsu
                    | IntOp::DivU
                    | IntOp::Div
                    | IntOp::RemU
                    | IntOp::Rem
                    | IntOp::SAddU
                    | IntOp::SAdd
                    | IntOp::SSubU
                    | IntOp::SSub
                    | IntOp::AAddU
                    | IntOp::AAdd
                    | IntOp::ASubU
                    | IntOp::ASub
                    | IntOp::Smul
                    | IntOp::SSrl
                    | IntOp::SSra
                    | IntOp::AndN
                    | IntOp::Rol
                    | IntOp::Ror
                    | IntOp::ClMul
                    | IntOp::ClMulH,
                )
                | VecAluOp::Macc(_)
                | VecAluOp::Merge
                | VecAluOp::Carry(CarryOp::Adc | CarryOp::Sbc),
            )
            | Self::Permute(
                PermuteOp::SlideUp(_)
                | PermuteOp::SlideDown(_)
                | PermuteOp::Slide1Up
                | PermuteOp::Slide1Down
                | PermuteOp::Rgather
                | PermuteOp::RgatherEi16
                | PermuteOp::Compress,
            )
            | Self::Fp(
                VecFpOp::Binary(_)
                | VecFpOp::Fma(_, _)
                | VecFpOp::Merge
                | VecFpOp::Slide1Up
                | VecFpOp::Slide1Down,
            ) => VecOperandGroups { vd: lmul, vs2: lmul, vs1: vs1_base },

            // Zvknh / Zvksh vsm3me / Zvkg vghsh: vs1 is a per-group vector input.
            Self::Crypto(
                CryptoOp::Sha2Ms
                | CryptoOp::Sha2Ch
                | CryptoOp::Sha2Cl
                | CryptoOp::Sm3Me
                | CryptoOp::Ghsh,
            ) => VecOperandGroups { vd: lmul, vs2: lmul, vs1: lmul },

            // AES/SM4 rounds: vs1 field is a sub-opcode, not a register; .vs broadcasts vs2 EMUL=1.
            Self::Crypto(
                CryptoOp::AesEm
                | CryptoOp::AesEf
                | CryptoOp::AesDm
                | CryptoOp::AesDf
                | CryptoOp::AesZ
                | CryptoOp::Sm4R,
            ) => VecOperandGroups { vd: lmul, vs2: vs2_crypto, vs1: 0 },

            // vs1 field is a sub-opcode/imm here, not a register reference.
            Self::Crypto(
                CryptoOp::AesKf1
                | CryptoOp::AesKf2
                | CryptoOp::Sm4K
                | CryptoOp::Sm3C
                | CryptoOp::Gmul,
            )
            | Self::Fp(VecFpOp::Unary(_))
            | Self::Load(VecLoadOp::IndexedOrdered | VecLoadOp::IndexedUnordered)
            | Self::Store(VecStoreOp::IndexedOrdered | VecStoreOp::IndexedUnordered)
            | Self::Alu(VecAluOp::Int(
                IntOp::Brev | IntOp::Brev8 | IntOp::Rev8 | IntOp::Clz | IntOp::Ctz | IntOp::CpopV,
            )) => VecOperandGroups { vd: lmul, vs2: lmul, vs1: 0 },

            // RVV 1.0 §14.1: reduction vd/vs1 are single registers; vs2 is the full LMUL group.
            Self::Reduce(_) => VecOperandGroups { vd: 1, vs2: lmul, vs1: 1 },
            Self::Alu(
                VecAluOp::Widen(
                    WidenOp::AddU
                    | WidenOp::Add
                    | WidenOp::SubU
                    | WidenOp::Sub
                    | WidenOp::MulU
                    | WidenOp::Mul
                    | WidenOp::MulSU
                    | WidenOp::Sll,
                )
                | VecAluOp::WidenMacc(_),
            )
            | Self::Fp(
                VecFpOp::Widen(FpWidenOp::Add | FpWidenOp::Sub | FpWidenOp::Mul)
                | VecFpOp::WidenFma(_),
            ) => VecOperandGroups { vd: emul_widened, vs2: lmul, vs1: vs1_base },
            Self::Alu(VecAluOp::Widen(
                WidenOp::AddUW | WidenOp::AddW | WidenOp::SubUW | WidenOp::SubW,
            ))
            | Self::Fp(VecFpOp::Widen(FpWidenOp::AddW | FpWidenOp::SubW)) => {
                VecOperandGroups { vd: emul_widened, vs2: emul_widened, vs1: vs1_base }
            }
            Self::Alu(VecAluOp::Narrow(_)) => {
                VecOperandGroups { vd: lmul, vs2: emul_widened, vs1: vs1_base }
            }

            // Use 1 as minimum; vs2 EMUL = LMUL/factor may be fractional (<1 register).
            Self::Alu(VecAluOp::Extend(op)) => {
                VecOperandGroups { vd: lmul, vs2: (lmul / op.factor.divisor()).max(1), vs1: 0 }
            }
            Self::Fp(VecFpOp::WidenConvert(_)) => {
                VecOperandGroups { vd: emul_widened, vs2: lmul, vs1: 0 }
            }
            Self::Fp(VecFpOp::NarrowConvert(_)) => {
                VecOperandGroups { vd: lmul, vs2: emul_widened, vs1: 0 }
            }

            // RVV 1.0 §16.1: vmv.x.s / vfmv.f.s read only element 0 of vs2.
            Self::Permute(PermuteOp::MvXS)
            | Self::Fp(VecFpOp::MoveToScalar)
            | Self::Mask(MaskOp::CPop | MaskOp::First) => {
                VecOperandGroups { vd: 0, vs2: 1, vs1: 0 }
            }

            // vmv.s.x / vfmv.s.f write only element 0 (§16.1); vlm/vsm: single mask register.
            Self::Permute(PermuteOp::MvSX)
            | Self::Fp(VecFpOp::MoveFromScalar)
            | Self::Load(VecLoadOp::Mask)
            | Self::Store(VecStoreOp::Mask) => VecOperandGroups { vd: 1, vs2: 0, vs1: 0 },
            Self::Alu(VecAluOp::Compare(_) | VecAluOp::Carry(CarryOp::Madc | CarryOp::Msbc))
            | Self::Fp(VecFpOp::Compare(_)) => VecOperandGroups { vd: 1, vs2: lmul, vs1: vs1_base },
            Self::Mask(MaskOp::Set(_)) => VecOperandGroups { vd: 1, vs2: 1, vs1: 0 },
            Self::Mask(MaskOp::Iota) => VecOperandGroups { vd: lmul, vs2: 1, vs1: 0 },

            // vid.v's vs2 field is part of the opcode (must be zero), not a register.
            Self::Mask(MaskOp::Id)
            | Self::Load(VecLoadOp::Unit | VecLoadOp::FaultOnlyFirst | VecLoadOp::Strided)
            | Self::Store(VecStoreOp::Unit | VecStoreOp::Strided) => {
                VecOperandGroups { vd: lmul, vs2: 0, vs1: 0 }
            }
            Self::Mask(MaskOp::Logical(_)) => VecOperandGroups { vd: 1, vs2: 1, vs1: 1 },
            Self::Permute(PermuteOp::WholeMove(registers)) => {
                VecOperandGroups { vd: registers.count(), vs2: registers.count(), vs1: 0 }
            }

            // nf encoding: 0=1reg, 1=2reg, 3=4reg, 7=8reg; group = nf+1.
            Self::Load(VecLoadOp::WholeRegister) | Self::Store(VecStoreOp::WholeRegister) => {
                let regs = nf + 1;
                VecOperandGroups { vd: regs, vs2: 0, vs1: 0 }
            }
        }
    }
}

/// Where a slide takes its element offset: the ISA allows `.vx` and `.vi`
/// forms only, so an executor never sees a vector operand here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlideOffset {
    /// The value of `rs1`.
    Rs1,
    /// A zero-extended 5-bit immediate.
    Imm(u8),
}

/// Element-wise integer operations: one result element per source element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntOp {
    /// `vadd`.
    Add,
    /// `vsub`.
    Sub,
    /// `vrsub`.
    Rsub,
    /// `vand`.
    And,
    /// `vor`.
    Or,
    /// `vxor`.
    Xor,
    /// `vsll`.
    Sll,
    /// `vsrl`.
    Srl,
    /// `vsra`.
    Sra,
    /// `vminu`.
    MinU,
    /// `vmin`.
    Min,
    /// `vmaxu`.
    MaxU,
    /// `vmax`.
    Max,
    /// `vmul`.
    Mul,
    /// `vmulh`.
    Mulh,
    /// `vmulhu`.
    Mulhu,
    /// `vmulhsu`.
    Mulhsu,
    /// `vdivu`.
    DivU,
    /// `vdiv`.
    Div,
    /// `vremu`.
    RemU,
    /// `vrem`.
    Rem,
    /// `vsaddu`.
    SAddU,
    /// `vsadd`.
    SAdd,
    /// `vssubu`.
    SSubU,
    /// `vssub`.
    SSub,
    /// `vaaddu`.
    AAddU,
    /// `vaadd`.
    AAdd,
    /// `vasubu`.
    ASubU,
    /// `vasub`.
    ASub,
    /// `vsmul`.
    Smul,
    /// `vssrl`.
    SSrl,
    /// `vssra`.
    SSra,
    /// `vandn`.
    AndN,
    /// `vbrev.v`.
    Brev,
    /// `vbrev8.v`.
    Brev8,
    /// `vrev8.v`.
    Rev8,
    /// `vclz.v`.
    Clz,
    /// `vctz.v`.
    Ctz,
    /// `vcpop.v`.
    CpopV,
    /// `vrol`.
    Rol,
    /// `vror`.
    Ror,
    /// `vclmul`.
    ClMul,
    /// `vclmulh`.
    ClMulH,
}

/// Integer comparisons that write a mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareOp {
    /// `vmseq`.
    Eq,
    /// `vmsne`.
    Ne,
    /// `vmsltu`.
    LtU,
    /// `vmslt`.
    Lt,
    /// `vmsleu`.
    LeU,
    /// `vmsle`.
    Le,
    /// `vmsgtu`.
    GtU,
    /// `vmsgt`.
    Gt,
}

/// Add and subtract with the carry in `v0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarryOp {
    /// `vadc`: the sum.
    Adc,
    /// `vmadc`: the carry out, as a mask.
    Madc,
    /// `vsbc`: the difference.
    Sbc,
    /// `vmsbc`: the borrow out, as a mask.
    Msbc,
}

impl CarryOp {
    /// True for the forms that write a mask rather than elements.
    #[must_use]
    pub const fn writes_mask(self) -> bool {
        matches!(self, Self::Madc | Self::Msbc)
    }
}

/// Multiply-accumulate at SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaccOp {
    /// `vmacc`: `vd = vs1 * vs2 + vd`.
    Macc,
    /// `vnmsac`: `vd = -(vs1 * vs2) + vd`.
    NMSac,
    /// `vmadd`: `vd = vs1 * vd + vs2`.
    Madd,
    /// `vnmsub`: `vd = -(vs1 * vd) + vs2`.
    NMSub,
}

/// Widening arithmetic: sources at SEW (or `vs2` at 2×SEW for the `.w`
/// forms), result at 2×SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidenOp {
    /// `vwaddu`.
    AddU,
    /// `vwadd`.
    Add,
    /// `vwsubu`.
    SubU,
    /// `vwsub`.
    Sub,
    /// `vwaddu.w`.
    AddUW,
    /// `vwadd.w`.
    AddW,
    /// `vwsubu.w`.
    SubUW,
    /// `vwsub.w`.
    SubW,
    /// `vwmulu`.
    MulU,
    /// `vwmul`.
    Mul,
    /// `vwmulsu`.
    MulSU,
    /// `vwsll`.
    Sll,
}

impl WidenOp {
    /// True for the `.w` forms, which read `vs2` at the wide width.
    #[must_use]
    pub const fn reads_wide_vs2(self) -> bool {
        matches!(self, Self::AddUW | Self::AddW | Self::SubUW | Self::SubW)
    }
}

/// Widening multiply-accumulate: product at 2×SEW added to `vd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidenMaccOp {
    /// `vwmaccu`.
    MaccU,
    /// `vwmacc`.
    Macc,
    /// `vwmaccsu`.
    MaccSU,
    /// `vwmaccus`.
    MaccUS,
}

/// Narrowing shifts and clips: `vs2` at 2×SEW, result at SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NarrowOp {
    /// `vnsrl`.
    Srl,
    /// `vnsra`.
    Sra,
    /// `vnclipu`.
    ClipU,
    /// `vnclip`.
    Clip,
}

/// Integer extension from a fraction of SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtendOp {
    /// Sign- or zero-extension.
    pub signedness: Signedness,
    /// The fraction of SEW the source is.
    pub factor: ExtendFactor,
}

/// The source width of an extension: SEW divided by 2, 4 or 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtendFactor {
    /// `vf2`.
    Vf2,
    /// `vf4`.
    Vf4,
    /// `vf8`.
    Vf8,
}

impl ExtendFactor {
    /// What SEW is divided by.
    #[must_use]
    pub const fn divisor(self) -> u8 {
        match self {
            Self::Vf2 => 2,
            Self::Vf4 => 4,
            Self::Vf8 => 8,
        }
    }
}

/// How many registers a whole-register move copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WholeRegisters {
    /// `vmv1r.v`.
    One,
    /// `vmv2r.v`.
    Two,
    /// `vmv4r.v`.
    Four,
    /// `vmv8r.v`.
    Eight,
}

impl WholeRegisters {
    /// The number of registers.
    #[must_use]
    pub const fn count(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Four => 4,
            Self::Eight => 8,
        }
    }
}

/// What the vector integer ALU computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecAluOp {
    /// An element-wise operation at SEW.
    Int(IntOp),
    /// A comparison into a mask.
    Compare(CompareOp),
    /// Add or subtract with carry.
    Carry(CarryOp),
    /// Multiply-accumulate.
    Macc(MaccOp),
    /// Widening arithmetic.
    Widen(WidenOp),
    /// Widening multiply-accumulate.
    WidenMacc(WidenMaccOp),
    /// Narrowing shift or clip.
    Narrow(NarrowOp),
    /// Zero- or sign-extension.
    Extend(ExtendOp),
    /// `vmerge` / `vmv.v`.
    Merge,
}

/// Single-width floating-point operations on `vs2` and `vs1` or `fs1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpBinaryOp {
    /// `vfadd`.
    Add,
    /// `vfsub`.
    Sub,
    /// `vfrsub`: `op1 - vs2`.
    RSub,
    /// `vfmul`.
    Mul,
    /// `vfdiv`.
    Div,
    /// `vfrdiv`: `op1 / vs2`.
    RDiv,
    /// `vfmin`.
    Min,
    /// `vfmax`.
    Max,
    /// `vfsgnj`.
    Sgnj,
    /// `vfsgnjn`.
    Sgnjn,
    /// `vfsgnjx`.
    Sgnjx,
}

/// How a conversion to an integer rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntRounding {
    /// Under `frm`.
    Dynamic,
    /// Toward zero (`.rtz`).
    TowardZero,
}

/// A conversion between floating point and an integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntConvert {
    /// Floating point to an integer.
    ToInt(Signedness, IntRounding),
    /// An integer to floating point.
    FromInt(Signedness),
}

/// Single-width floating-point operations on `vs2` alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpUnaryOp {
    /// `vfsqrt`.
    Sqrt,
    /// `vfrsqrt7`.
    Rsqrt7,
    /// `vfrec7`.
    Rec7,
    /// `vfclass`.
    Class,
    /// `vfcvt`.
    Convert(IntConvert),
}

/// Which register a single-width fused multiply-add adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FmaAddend {
    /// `vd`, as `vfmacc` and its kin accumulate.
    Vd,
    /// `vs2`, as `vfmadd` and its kin, which multiply `vd`.
    Vs2,
}

/// Widening floating-point arithmetic: a 2×SEW result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpWidenOp {
    /// `vfwadd`.
    Add,
    /// `vfwsub`.
    Sub,
    /// `vfwmul`.
    Mul,
    /// `vfwadd.w`: `vs2` is already 2×SEW.
    AddW,
    /// `vfwsub.w`: `vs2` is already 2×SEW.
    SubW,
}

/// Widening conversions: a 2×SEW result from a SEW source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpWidenConvertOp {
    /// To or from an integer.
    Int(IntConvert),
    /// `vfwcvt.f.f`.
    Float,
}

/// Narrowing conversions: a SEW result from a 2×SEW source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpNarrowConvertOp {
    /// To or from an integer.
    Int(IntConvert),
    /// `vfncvt.f.f`.
    Float,
    /// `vfncvt.rod.f.f`: rounding to odd.
    FloatRoundToOdd,
}

/// The vector floating-point operations, by how they compute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecFpOp {
    /// Single-width, from `vs2` and `vs1` or `fs1`.
    Binary(FpBinaryOp),
    /// Single-width, from `vs2` alone.
    Unary(FpUnaryOp),
    /// Single-width fused multiply-add.
    Fma(FmaSigns, FmaAddend),
    /// A comparison into a mask.
    Compare(FpCompareOp),
    /// Widening arithmetic.
    Widen(FpWidenOp),
    /// Widening fused multiply-accumulate into `vd`.
    WidenFma(FmaSigns),
    /// Widening conversion.
    WidenConvert(FpWidenConvertOp),
    /// Narrowing conversion.
    NarrowConvert(FpNarrowConvertOp),
    /// `vfmerge`.
    Merge,
    /// `vfmv.f.s`.
    MoveToScalar,
    /// `vfmv.s.f`.
    MoveFromScalar,
    /// `vfslide1up`.
    Slide1Up,
    /// `vfslide1down`.
    Slide1Down,
}

/// Integer reductions at SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntReduceOp {
    /// `vredsum`.
    Sum,
    /// `vredand`.
    And,
    /// `vredor`.
    Or,
    /// `vredxor`.
    Xor,
    /// `vredminu`.
    MinU,
    /// `vredmin`.
    Min,
    /// `vredmaxu`.
    MaxU,
    /// `vredmax`.
    Max,
}

/// Widening integer reductions: sources at SEW, accumulator at 2×SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidenIntReduceOp {
    /// `vwredsumu`.
    SumU,
    /// `vwredsum`.
    Sum,
}

/// Floating-point reductions at SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpReduceOp {
    /// `vfredosum`.
    OSum,
    /// `vfredusum`.
    USum,
    /// `vfredmax`.
    Max,
    /// `vfredmin`.
    Min,
}

/// Widening floating-point reductions: sources at SEW, accumulator at 2×SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpWidenReduceOp {
    /// `vfwredosum`.
    OSum,
    /// `vfwredusum`.
    USum,
}

/// A reduction of `vs2` into element 0 of `vd`, seeded from `vs1[0]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReduceOp {
    /// Integer, at SEW.
    Int(IntReduceOp),
    /// Integer, widening.
    WidenInt(WidenIntReduceOp),
    /// Floating-point, at SEW.
    Fp(FpReduceOp),
    /// Floating-point, widening.
    FpWiden(FpWidenReduceOp),
}

/// Bitwise operations between mask registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskLogicalOp {
    /// `vmand.mm`.
    And,
    /// `vmnand.mm`.
    Nand,
    /// `vmandn.mm`.
    AndNot,
    /// `vmor.mm`.
    Or,
    /// `vmnor.mm`.
    Nor,
    /// `vmorn.mm`.
    OrNot,
    /// `vmxor.mm`.
    Xor,
    /// `vmxnor.mm`.
    Xnor,
}

/// The set-before/including/only-first mask operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskSetOp {
    /// `vmsbf.m`: set every bit before the first set source bit.
    BeforeFirst,
    /// `vmsif.m`: set every bit up to and including the first set source bit.
    IncludingFirst,
    /// `vmsof.m`: set only the first set source bit.
    OnlyFirst,
}

/// Operations on mask registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskOp {
    /// A bitwise operation between two masks.
    Logical(MaskLogicalOp),
    /// `vcpop.m`: count the set bits.
    CPop,
    /// `vfirst.m`: index of the first set bit.
    First,
    /// `vmsbf.m`, `vmsif.m` or `vmsof.m`.
    Set(MaskSetOp),
    /// `viota.m`.
    Iota,
    /// `vid.v`.
    Id,
}

/// Permutations: moves, slides, gathers and compress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermuteOp {
    /// `vmv.x.s`.
    MvXS,
    /// `vmv.s.x`.
    MvSX,
    /// `vslideup` by the offset.
    SlideUp(SlideOffset),
    /// `vslidedown` by the offset.
    SlideDown(SlideOffset),
    /// `vslide1up`.
    Slide1Up,
    /// `vslide1down`.
    Slide1Down,
    /// `vrgather`.
    Rgather,
    /// `vrgatherei16`.
    RgatherEi16,
    /// `vcompress`.
    Compress,
    /// `vmv<n>r.v`: a whole-register move.
    WholeMove(WholeRegisters),
}

/// Vector cryptography operations (Zvkned, Zvknha/b, Zvksed, Zvksh, Zvkg).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptoOp {
    /// `vaesem`.
    AesEm,
    /// `vaesef`.
    AesEf,
    /// `vaesdm`.
    AesDm,
    /// `vaesdf`.
    AesDf,
    /// `vaesz`.
    AesZ,
    /// `vaeskf1`.
    AesKf1,
    /// `vaeskf2`.
    AesKf2,
    /// `vsha2ms`.
    Sha2Ms,
    /// `vsha2ch`.
    Sha2Ch,
    /// `vsha2cl`.
    Sha2Cl,
    /// `vsm3me`.
    Sm3Me,
    /// `vsm3c`.
    Sm3C,
    /// `vsm4r`.
    Sm4R,
    /// `vsm4k`.
    Sm4K,
    /// `vgmul`.
    Gmul,
    /// `vghsh`.
    Ghsh,
}

/// Vector operand source encoding category.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VecSrcEncoding {
    /// Not a vector source encoding.
    #[default]
    None,
    /// Vector-vector (OPIVV, OPFVV, OPMVV).
    VV,
    /// Vector-scalar integer (OPIVX, OPMVX).
    VX,
    /// Vector-immediate (OPIVI), with the immediate as the operation
    /// reads it: `simm5`, `uimm5`, or `vror`'s 6-bit `zimm`.
    VI(i8),
    /// Vector-scalar FP (OPFVF).
    VF,
}
