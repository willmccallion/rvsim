//! Pipeline control signals and operation types.
//!
//! This module defines the signals that control instruction execution. It performs:
//! 1. **Operation Classification:** Categorizes ALU, atomic, and CSR operations.
//! 2. **Operand Selection:** Defines sources for ALU inputs (registers, PC, or immediates).
//! 3. **Memory Control:** Specifies access widths and sign-extension requirements.
//! 4. **System Control:** Manages privilege transitions and system-level instructions.

use crate::common::CsrAddr;
use crate::core::units::vpu::types::{Sew, VRegIdx};

/// ALU operation types for integer and floating-point instructions.
#[derive(Clone, Copy, Debug, Default)]
pub enum AluOp {
    /// Default value (no operation).
    #[default]
    Add,

    /// Integer subtraction.
    Sub,

    /// Shift left logical.
    Sll,

    /// Set less than (signed).
    Slt,

    /// Set less than unsigned.
    Sltu,

    /// Bitwise XOR.
    Xor,

    /// Shift right logical.
    Srl,

    /// Shift right arithmetic.
    Sra,

    /// Bitwise OR.
    Or,

    /// Bitwise AND.
    And,

    /// Integer multiply (low bits).
    Mul,

    /// Integer multiply (high bits, signed × signed).
    Mulh,

    /// Integer multiply (high bits, signed × unsigned).
    Mulhsu,

    /// Integer multiply (high bits, unsigned × unsigned).
    Mulhu,

    /// Integer divide (signed).
    Div,

    /// Integer divide (unsigned).
    Divu,

    /// Integer remainder (signed).
    Rem,

    /// Integer remainder (unsigned).
    Remu,

    /// Floating-point addition.
    FAdd,

    /// Floating-point subtraction.
    FSub,

    /// Floating-point multiplication.
    FMul,

    /// Floating-point division.
    FDiv,

    /// Floating-point square root.
    FSqrt,

    /// Floating-point minimum.
    FMin,

    /// Floating-point maximum.
    FMax,

    /// Floating-point multiply-add (fused).
    FMAdd,

    /// Floating-point multiply-subtract (fused).
    FMSub,

    /// Floating-point negated multiply-add (fused).
    FNMAdd,

    /// Floating-point negated multiply-subtract (fused).
    FNMSub,

    /// Convert word to single-precision float (signed).
    FCvtWS,

    /// Convert long to single-precision float (signed).
    FCvtLS,

    /// Convert single-precision float to word (signed).
    FCvtSW,

    /// Convert single-precision float to long (signed).
    FCvtSL,

    /// Convert float to word (unsigned).
    FCvtWUS,

    /// Convert float to long (unsigned).
    FCvtLUS,

    /// Convert unsigned word to float.
    FCvtSWU,

    /// Convert unsigned long to float.
    FCvtSLU,

    /// Convert single-precision to double-precision float.
    FCvtSD,

    /// Convert double-precision to single-precision float.
    FCvtDS,

    /// Floating-point sign injection (copy sign).
    FSgnJ,

    /// Floating-point sign injection (negate sign).
    FSgnJN,

    /// Floating-point sign injection (XOR sign).
    FSgnJX,

    /// Floating-point equality comparison.
    FEq,

    /// Floating-point less-than comparison.
    FLt,

    /// Floating-point less-than-or-equal comparison.
    FLe,

    /// Floating-point classify.
    FClass,

    /// Move floating-point register to integer register.
    FMvToX,

    /// Move integer register to floating-point register.
    FMvToF,
}

/// Atomic memory operation types (RISC-V A extension).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AtomicOp {
    /// No atomic operation.
    #[default]
    None,

    /// Load-reserved (atomic load with reservation).
    Lr,

    /// Store-conditional (atomic store if reservation valid).
    Sc,

    /// Atomic swap.
    Swap,

    /// Atomic add.
    Add,

    /// Atomic XOR.
    Xor,

    /// Atomic AND.
    And,

    /// Atomic OR.
    Or,

    /// Atomic minimum (signed).
    Min,

    /// Atomic maximum (signed).
    Max,

    /// Atomic minimum (unsigned).
    Minu,

    /// Atomic maximum (unsigned).
    Maxu,
}

/// Memory access width for load and store operations.
#[derive(Clone, Copy, Debug, Default)]
pub enum MemWidth {
    /// No memory operation.
    #[default]
    Nop,

    /// 8-bit byte access.
    Byte,

    /// 16-bit half-word access.
    Half,

    /// 32-bit word access.
    Word,

    /// 64-bit double-word access.
    Double,
}

/// Source for ALU operand A.
#[derive(Clone, Copy, Debug, Default)]
pub enum OpASrc {
    /// Use `rs1` register value.
    #[default]
    Reg1,

    /// Use program counter value.
    Pc,

    /// Use zero.
    Zero,
}

/// Source for ALU operand B.
#[derive(Clone, Copy, Debug, Default)]
pub enum OpBSrc {
    /// Use sign-extended immediate value.
    #[default]
    Imm,

    /// Use `rs2` register value.
    Reg2,

    /// Use zero.
    Zero,
}

/// Control flow classification for pipeline instructions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlFlow {
    /// Sequential instruction (no branch or jump).
    #[default]
    Sequential,

    /// Conditional branch instruction.
    Branch,

    /// Unconditional jump (`JAL`/`JALR`).
    Jump,
}

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

    /// Generic system instruction (CSR, ECALL) not covered by a specific variant.
    System,
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

/// Control signals for pipeline stage execution.
///
/// Contains all signals generated during instruction decode that control execution
/// and memory access throughout the pipeline stages.
#[derive(Clone, Copy, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct ControlSignals {
    /// Enable write to integer destination register.
    pub reg_write: bool,
    /// Enable write to floating-point destination register.
    pub fp_reg_write: bool,
    /// Enable memory read operation (load).
    pub mem_read: bool,
    /// Enable memory write operation (store).
    pub mem_write: bool,
    /// Control flow type (sequential, branch, or jump).
    pub control_flow: ControlFlow,
    /// Instruction uses 32-bit operands.
    pub is_rv32: bool,
    /// Width of memory access.
    pub width: MemWidth,
    /// Load should be sign-extended.
    pub signed_load: bool,
    /// ALU operation to perform.
    pub alu: AluOp,
    /// Source selection for ALU operand A.
    pub a_src: OpASrc,
    /// Source selection for ALU operand B.
    pub b_src: OpBSrc,
    /// System operation type.
    pub system_op: SystemOp,
    /// CSR address for CSR operations.
    pub csr_addr: CsrAddr,
    /// CSR operation type.
    pub csr_op: CsrOp,
    /// `rs1` is a floating-point register.
    pub rs1_fp: bool,
    /// `rs2` is a floating-point register.
    pub rs2_fp: bool,
    /// `rs3` is a floating-point register.
    pub rs3_fp: bool,
    /// Atomic memory operation type.
    pub atomic_op: AtomicOp,
    /// Vector operation type.
    pub vec_op: VectorOp,
    /// Vector destination register.
    pub vd: VRegIdx,
    /// Vector source register 1.
    pub vs1: VRegIdx,
    /// Vector source register 2.
    pub vs2: VRegIdx,
    /// Vector source register 3 (FMA, stores).
    pub vs3: VRegIdx,
    /// Masking bit (true = unmasked).
    pub vm: bool,
    /// Enable write to vector destination register.
    pub vec_reg_write: bool,
    /// Vector source encoding category.
    pub vec_src_encoding: VecSrcEncoding,
    /// Effective element width for vector loads/stores.
    pub vec_eew: Sew,
    /// Segment field count minus 1 (nf encoding: 0 = 1 field, 7 = 8 fields).
    pub vec_nf: u8,
    /// Number of registers in the LMUL group (1, 2, 4, or 8).
    /// Derived from LMUL at decode time. 0 for non-vector instructions.
    pub vec_lmul_regs: u8,
}

/// Vector operation type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VectorOp {
    /// No vector operation.
    #[default]
    None,

    // ── Configuration ────────────────────────────────────────────────────
    /// `vsetvli` — set vl/vtype from rs1 and immediate.
    Vsetvli,
    /// `vsetivli` — set vl/vtype from uimm and immediate.
    Vsetivli,
    /// `vsetvl` — set vl/vtype from rs1 and rs2.
    Vsetvl,

    // ── Integer arithmetic ───────────────────────────────────────────────
    /// `vadd` — vector add.
    VAdd,
    /// `vsub` — vector subtract.
    VSub,
    /// `vrsub` — vector reverse subtract (imm/scalar - vs2).
    VRsub,
    /// `vand` — vector bitwise AND.
    VAnd,
    /// `vor` — vector bitwise OR.
    VOr,
    /// `vxor` — vector bitwise XOR.
    VXor,
    /// `vsll` — vector shift left logical.
    VSll,
    /// `vsrl` — vector shift right logical.
    VSrl,
    /// `vsra` — vector shift right arithmetic.
    VSra,
    /// `vminu` — vector unsigned minimum.
    VMinU,
    /// `vmin` — vector signed minimum.
    VMin,
    /// `vmaxu` — vector unsigned maximum.
    VMaxU,
    /// `vmax` — vector signed maximum.
    VMax,

    // ── Merge / move ─────────────────────────────────────────────────────
    /// `vmerge` / `vmv` — vector merge or move.
    VMerge,

    // ── Integer comparison (write mask) ──────────────────────────────────
    /// `vmseq` — set mask if equal.
    VMSeq,
    /// `vmsne` — set mask if not equal.
    VMSne,
    /// `vmsltu` — set mask if less than unsigned.
    VMSltu,
    /// `vmslt` — set mask if less than signed.
    VMSlt,
    /// `vmsleu` — set mask if less than or equal unsigned.
    VMSleu,
    /// `vmsle` — set mask if less than or equal signed.
    VMSle,
    /// `vmsgtu` — set mask if greater than unsigned.
    VMSgtu,
    /// `vmsgt` — set mask if greater than signed.
    VMSgt,

    // ── Add/subtract with carry ──────────────────────────────────────────
    /// `vadc` — add with carry from v0 mask.
    VAdc,
    /// `vmadc` — mask-producing add with carry.
    VMadc,
    /// `vsbc` — subtract with borrow from v0 mask.
    VSbc,
    /// `vmsbc` — mask-producing subtract with borrow.
    VMsbc,

    // ── Integer multiply ─────────────────────────────────────────────────
    /// `vmul` — multiply low bits.
    VMul,
    /// `vmulh` — multiply high bits (signed × signed).
    VMulh,
    /// `vmulhu` — multiply high bits (unsigned × unsigned).
    VMulhu,
    /// `vmulhsu` — multiply high bits (signed × unsigned).
    VMulhsu,
    /// `vmacc` — multiply-accumulate (vd = vs1*vs2 + vd).
    VMacc,
    /// `vnmsac` — negated multiply-subtract accumulate (vd = -(vs1*vs2) + vd).
    VNMSac,
    /// `vmadd` — multiply-add (vd = vs1*vd + vs2).
    VMadd,
    /// `vnmsub` — negated multiply-subtract (vd = -(vs1*vd) + vs2).
    VNMSub,

    // ── Integer divide ───────────────────────────────────────────────────
    /// `vdivu` — unsigned divide.
    VDivU,
    /// `vdiv` — signed divide.
    VDiv,
    /// `vremu` — unsigned remainder.
    VRemU,
    /// `vrem` — signed remainder.
    VRem,

    // ── Widening integer arithmetic ──────────────────────────────────────
    /// `vwaddu` — widening unsigned add (SEW → 2×SEW).
    VWAddU,
    /// `vwadd` — widening signed add (SEW → 2×SEW).
    VWAdd,
    /// `vwsubu` — widening unsigned subtract (SEW → 2×SEW).
    VWSubU,
    /// `vwsub` — widening signed subtract (SEW → 2×SEW).
    VWSub,
    /// `vwaddu.w` — widening unsigned add wide (2×SEW op SEW → 2×SEW).
    VWAddUW,
    /// `vwadd.w` — widening signed add wide (2×SEW op SEW → 2×SEW).
    VWAddW,
    /// `vwsubu.w` — widening unsigned subtract wide (2×SEW op SEW → 2×SEW).
    VWSubUW,
    /// `vwsub.w` — widening signed subtract wide (2×SEW op SEW → 2×SEW).
    VWSubW,

    // ── Widening integer multiply ────────────────────────────────────────
    /// `vwmulu` — widening unsigned multiply.
    VWMulU,
    /// `vwmul` — widening signed multiply.
    VWMul,
    /// `vwmulsu` — widening signed-unsigned multiply.
    VWMulSU,
    /// `vwmaccu` — widening unsigned multiply-accumulate.
    VWMaccU,
    /// `vwmacc` — widening signed multiply-accumulate.
    VWMacc,
    /// `vwmaccsu` — widening signed-unsigned multiply-accumulate.
    VWMaccSU,
    /// `vwmaccus` — widening unsigned-signed multiply-accumulate.
    VWMaccUS,

    // ── Narrowing ────────────────────────────────────────────────────────
    /// `vnsrl` — narrowing shift right logical (2×SEW → SEW).
    VNSrl,
    /// `vnsra` — narrowing shift right arithmetic (2×SEW → SEW).
    VNSra,
    /// `vnclipu` — narrowing clip unsigned with saturation.
    VNClipU,
    /// `vnclip` — narrowing clip signed with saturation.
    VNClip,

    // ── Fixed-point saturating ───────────────────────────────────────────
    /// `vsaddu` — saturating unsigned add.
    VSAddU,
    /// `vsadd` — saturating signed add.
    VSAdd,
    /// `vssubu` — saturating unsigned subtract.
    VSSubU,
    /// `vssub` — saturating signed subtract.
    VSSub,

    // ── Fixed-point averaging ────────────────────────────────────────────
    /// `vaaddu` — averaging unsigned add.
    VAAddU,
    /// `vaadd` — averaging signed add.
    VAAdd,
    /// `vasubu` — averaging unsigned subtract.
    VASubU,
    /// `vasub` — averaging signed subtract.
    VASub,

    // ── Fixed-point scaling ──────────────────────────────────────────────
    /// `vsmul` — signed fractional multiply with rounding.
    VSmul,
    /// `vssrl` — scaling shift right logical with rounding.
    VSSrl,
    /// `vssra` — scaling shift right arithmetic with rounding.
    VSSra,

    // ── Extension ────────────────────────────────────────────────────────
    /// `vzext.vf2` — zero-extend SEW/2 to SEW.
    VZextVf2,
    /// `vzext.vf4` — zero-extend SEW/4 to SEW.
    VZextVf4,
    /// `vzext.vf8` — zero-extend SEW/8 to SEW.
    VZextVf8,
    /// `vsext.vf2` — sign-extend SEW/2 to SEW.
    VSextVf2,
    /// `vsext.vf4` — sign-extend SEW/4 to SEW.
    VSextVf4,
    /// `vsext.vf8` — sign-extend SEW/8 to SEW.
    VSextVf8,

    // ── Vector memory — unit-stride ──────────────────────────────────────
    /// Unit-stride vector load (`vle8/16/32/64`).
    VLoadUnit,
    /// Unit-stride vector store (`vse8/16/32/64`).
    VStoreUnit,
    /// Fault-only-first vector load (`vle8ff/16ff/32ff/64ff`).
    VLoadFF,
    /// Mask load (`vlm.v`).
    VLoadMask,
    /// Mask store (`vsm.v`).
    VStoreMask,
    /// Whole-register load (`vl1re8`, `vl2re8`, etc.).
    VLoadWholeReg,
    /// Whole-register store (`vs1r`, `vs2r`, etc.).
    VStoreWholeReg,

    // ── Vector memory — strided ──────────────────────────────────────────
    /// Strided vector load (`vlse8/16/32/64`).
    VLoadStride,
    /// Strided vector store (`vsse8/16/32/64`).
    VStoreStride,

    // ── Vector memory — indexed ──────────────────────────────────────────
    /// Indexed ordered vector load (`vloxei8/16/32/64`).
    VLoadIndexOrd,
    /// Indexed ordered vector store (`vsoxei8/16/32/64`).
    VStoreIndexOrd,
    /// Indexed unordered vector load (`vluxei8/16/32/64`).
    VLoadIndexUnord,
    /// Indexed unordered vector store (`vsuxei8/16/32/64`).
    VStoreIndexUnord,

    // ── FP arithmetic ───────────────────────────────────────────────────
    /// `vfadd` — vector FP add.
    VFAdd,
    /// `vfsub` — vector FP subtract.
    VFSub,
    /// `vfrsub` — vector FP reverse subtract (scalar - vs2).
    VFRSub,
    /// `vfmul` — vector FP multiply.
    VFMul,
    /// `vfdiv` — vector FP divide.
    VFDiv,
    /// `vfrdiv` — vector FP reverse divide (scalar / vs2).
    VFRDiv,

    // ── FP min/max ──────────────────────────────────────────────────────
    /// `vfmin` — vector FP minimum.
    VFMin,
    /// `vfmax` — vector FP maximum.
    VFMax,

    // ── FP sign injection ───────────────────────────────────────────────
    /// `vfsgnj` — vector FP sign injection (copy sign).
    VFSgnj,
    /// `vfsgnjn` — vector FP negated sign injection.
    VFSgnjn,
    /// `vfsgnjx` — vector FP XOR sign injection.
    VFSgnjx,

    // ── FP comparison (write mask) ──────────────────────────────────────
    /// `vmfeq` — set mask if FP equal.
    VMFEq,
    /// `vmfne` — set mask if FP not equal.
    VMFNe,
    /// `vmflt` — set mask if FP less than.
    VMFLt,
    /// `vmfle` — set mask if FP less than or equal.
    VMFLe,
    /// `vmfgt` — set mask if FP greater than.
    VMFGt,
    /// `vmfge` — set mask if FP greater than or equal.
    VMFGe,

    // ── FP fused multiply-add ───────────────────────────────────────────
    /// `vfmacc` — FP multiply-accumulate (vd = vs1*vs2 + vd).
    VFMacc,
    /// `vfnmacc` — FP negated multiply-accumulate (vd = -(vs1*vs2) - vd).
    VFNMacc,
    /// `vfmsac` — FP multiply-subtract accumulate (vd = vs1*vs2 - vd).
    VFMSac,
    /// `vfnmsac` — FP negated multiply-subtract accumulate (vd = -(vs1*vs2) + vd).
    VFNMSac,
    /// `vfmadd` — FP multiply-add (vd = vs1*vd + vs2).
    VFMAdd,
    /// `vfnmadd` — FP negated multiply-add (vd = -(vs1*vd) - vs2).
    VFNMAdd,
    /// `vfmsub` — FP multiply-subtract (vd = vs1*vd - vs2).
    VFMSub,
    /// `vfnmsub` — FP negated multiply-subtract (vd = -(vs1*vd) + vs2).
    VFNMSub,

    // ── FP unary ────────────────────────────────────────────────────────
    /// `vfsqrt` — vector FP square root.
    VFSqrt,
    /// `vfrsqrt7` — vector FP reciprocal square root (7-bit accuracy).
    VFRsqrt7,
    /// `vfrec7` — vector FP reciprocal (7-bit accuracy).
    VFRec7,
    /// `vfclass` — vector FP classify.
    VFClass,

    // ── FP conversion (int<->float) ─────────────────────────────────────
    /// `vfcvt.xu.f` — convert FP to unsigned integer.
    VFCvtXuF,
    /// `vfcvt.x.f` — convert FP to signed integer.
    VFCvtXF,
    /// `vfcvt.f.xu` — convert unsigned integer to FP.
    VFCvtFXu,
    /// `vfcvt.f.x` — convert signed integer to FP.
    VFCvtFX,
    /// `vfcvt.rtz.xu.f` — convert FP to unsigned integer (round toward zero).
    VFCvtRtzXuF,
    /// `vfcvt.rtz.x.f` — convert FP to signed integer (round toward zero).
    VFCvtRtzXF,

    // ── FP widening arithmetic ──────────────────────────────────────────
    /// `vfwadd` — widening FP add (SEW -> 2*SEW).
    VFWAdd,
    /// `vfwsub` — widening FP subtract (SEW -> 2*SEW).
    VFWSub,
    /// `vfwmul` — widening FP multiply (SEW -> 2*SEW).
    VFWMul,
    /// `vfwadd.w` — widening FP add wide (2*SEW op SEW -> 2*SEW).
    VFWAddW,
    /// `vfwsub.w` — widening FP subtract wide (2*SEW op SEW -> 2*SEW).
    VFWSubW,

    // ── FP widening FMA ─────────────────────────────────────────────────
    /// `vfwmacc` — widening FP multiply-accumulate.
    VFWMacc,
    /// `vfwnmacc` — widening FP negated multiply-accumulate.
    VFWNMacc,
    /// `vfwmsac` — widening FP multiply-subtract accumulate.
    VFWMSac,
    /// `vfwnmsac` — widening FP negated multiply-subtract accumulate.
    VFWNMSac,

    // ── FP widening conversion ──────────────────────────────────────────
    /// `vfwcvt.xu.f` — widening convert FP to unsigned integer.
    VFWCvtXuF,
    /// `vfwcvt.x.f` — widening convert FP to signed integer.
    VFWCvtXF,
    /// `vfwcvt.f.xu` — widening convert unsigned integer to FP.
    VFWCvtFXu,
    /// `vfwcvt.f.x` — widening convert signed integer to FP.
    VFWCvtFX,
    /// `vfwcvt.f.f` — widening convert FP to wider FP.
    VFWCvtFF,
    /// `vfwcvt.rtz.xu.f` — widening convert FP to unsigned integer (round toward zero).
    VFWCvtRtzXuF,
    /// `vfwcvt.rtz.x.f` — widening convert FP to signed integer (round toward zero).
    VFWCvtRtzXF,

    // ── FP narrowing conversion ─────────────────────────────────────────
    /// `vfncvt.xu.f` — narrowing convert FP to unsigned integer.
    VFNCvtXuF,
    /// `vfncvt.x.f` — narrowing convert FP to signed integer.
    VFNCvtXF,
    /// `vfncvt.f.xu` — narrowing convert unsigned integer to FP.
    VFNCvtFXu,
    /// `vfncvt.f.x` — narrowing convert signed integer to FP.
    VFNCvtFX,
    /// `vfncvt.f.f` — narrowing convert FP to narrower FP.
    VFNCvtFF,
    /// `vfncvt.rod.f.f` — narrowing convert FP to narrower FP (round-odd).
    VFNCvtRodFF,
    /// `vfncvt.rtz.xu.f` — narrowing convert FP to unsigned integer (round toward zero).
    VFNCvtRtzXuF,
    /// `vfncvt.rtz.x.f` — narrowing convert FP to signed integer (round toward zero).
    VFNCvtRtzXF,

    // ── FP merge/move ───────────────────────────────────────────────────
    /// `vfmerge` — vector FP merge with mask.
    VFMerge,
    /// `vfmv.s.f` — move FP scalar to vector element 0.
    VFMvSF,
    /// `vfmv.f.s` — move vector element 0 to FP scalar.
    VFMvFS,

    // ── FP slides ───────────────────────────────────────────────────────
    /// `vfslide1up` — slide up by one with FP scalar.
    VFSlide1Up,
    /// `vfslide1down` — slide down by one with FP scalar.
    VFSlide1Down,

    // ── Integer reductions ──────────────────────────────────────────────
    /// `vredsum` — reduction sum.
    VRedSum,
    /// `vredand` — reduction AND.
    VRedAnd,
    /// `vredor` — reduction OR.
    VRedOr,
    /// `vredxor` — reduction XOR.
    VRedXor,
    /// `vredminu` — reduction unsigned minimum.
    VRedMinU,
    /// `vredmin` — reduction signed minimum.
    VRedMin,
    /// `vredmaxu` — reduction unsigned maximum.
    VRedMaxU,
    /// `vredmax` — reduction signed maximum.
    VRedMax,

    // ── Widening integer reductions ─────────────────────────────────────
    /// `vwredsumu` — widening unsigned reduction sum.
    VWRedSumU,
    /// `vwredsum` — widening signed reduction sum.
    VWRedSum,

    // ── FP reductions ───────────────────────────────────────────────────
    /// `vfredosum` — FP ordered reduction sum.
    VFRedOSum,
    /// `vfredusum` — FP unordered reduction sum.
    VFRedUSum,
    /// `vfredmax` — FP reduction maximum.
    VFRedMax,
    /// `vfredmin` — FP reduction minimum.
    VFRedMin,

    // ── FP widening reductions ──────────────────────────────────────────
    /// `vfwredosum` — widening FP ordered reduction sum.
    VFWRedOSum,
    /// `vfwredusum` — widening FP unordered reduction sum.
    VFWRedUSum,

    // ── Mask-register logical ───────────────────────────────────────────
    /// `vmand.mm` — mask AND.
    VMAndMM,
    /// `vmnand.mm` — mask NAND.
    VMNandMM,
    /// `vmandn.mm` — mask AND-NOT.
    VMAndnMM,
    /// `vmor.mm` — mask OR.
    VMOrMM,
    /// `vmnor.mm` — mask NOR.
    VMNorMM,
    /// `vmorn.mm` — mask OR-NOT.
    VMOrnMM,
    /// `vmxor.mm` — mask XOR.
    VMXorMM,
    /// `vmxnor.mm` — mask XNOR.
    VMXnorMM,

    // ── Mask scalar ─────────────────────────────────────────────────────
    /// `vcpop.m` — count population of mask register.
    VCPopM,
    /// `vfirst.m` — find first set bit in mask register.
    VFirstM,

    // ── Mask-producing ──────────────────────────────────────────────────
    /// `vmsbf.m` — set-before-first mask bit.
    VMSbfM,
    /// `vmsif.m` — set-including-first mask bit.
    VMSifM,
    /// `vmsof.m` — set-only-first mask bit.
    VMSofM,

    // ── Mask misc ───────────────────────────────────────────────────────
    /// `viota.m` — iota (prefix sum of mask bits).
    VIotaM,
    /// `vid.v` — vector element index.
    VIdV,

    // ── Permutations ────────────────────────────────────────────────────
    /// `vmv.x.s` — move vector element 0 to scalar GPR.
    VMvXS,
    /// `vmv.s.x` — move scalar GPR to vector element 0.
    VMvSX,
    /// `vslideup` — slide elements up.
    VSlideUp,
    /// `vslidedown` — slide elements down.
    VSlideDown,
    /// `vslide1up` — slide up by one with scalar.
    VSlide1Up,
    /// `vslide1down` — slide down by one with scalar.
    VSlide1Down,
    /// `vrgather` — register gather (permute by index).
    VRgather,
    /// `vrgatherei16` — register gather with 16-bit indices.
    VRgatherEi16,
    /// `vcompress` — compress active elements.
    VCompress,
    /// `vmv1r` — whole-register move (1 register).
    VMv1r,
    /// `vmv2r` — whole-register move (2 registers).
    VMv2r,
    /// `vmv4r` — whole-register move (4 registers).
    VMv4r,
    /// `vmv8r` — whole-register move (8 registers).
    VMv8r,
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

impl VectorOp {
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
    pub fn operand_groups(self, lmul: u8, src_enc: VecSrcEncoding) -> VecOperandGroups {
        use VectorOp::*;

        // vs1 is only a vector register for VV encoding; for VX/VI/VF it's a
        // scalar or immediate, so group size = 0.
        let vs1_base = if src_enc == VecSrcEncoding::VV { lmul } else { 0 };

        match self {
            // ── Configuration (no vector register operands) ──────────────
            None | Vsetvli | Vsetivli | Vsetvl => VecOperandGroups { vd: 0, vs1: 0, vs2: 0 },

            // ── Standard arithmetic (vd=LMUL, vs2=LMUL, vs1=LMUL|0) ────
            VAdd | VSub | VRsub | VAnd | VOr | VXor |
            VSll | VSrl | VSra |
            VMinU | VMin | VMaxU | VMax |
            VMul | VMulh | VMulhu | VMulhsu |
            VMacc | VNMSac | VMadd | VNMSub |
            VDivU | VDiv | VRemU | VRem |
            VSAddU | VSAdd | VSSubU | VSSub |
            VAAddU | VAAdd | VASubU | VASub |
            VSmul | VSSrl | VSSra |
            VMerge | VIdV |
            // Slides, gather, compress read full groups
            VSlideUp | VSlideDown | VSlide1Up | VSlide1Down |
            VRgather | VRgatherEi16 | VCompress |
            // FP standard arithmetic
            VFAdd | VFSub | VFRSub | VFMul | VFDiv | VFRDiv |
            VFMin | VFMax | VFSgnj | VFSgnjn | VFSgnjx |
            VFMacc | VFNMacc | VFMSac | VFNMSac |
            VFMAdd | VFNMAdd | VFMSub | VFNMSub |
            VFMerge |
            // FP slides
            VFSlide1Up | VFSlide1Down |
            // Reductions: vd is written as single element but still occupies
            // a single register (not a group), vs2 is the full LMUL source.
            // However, the spec says vd is treated as a single vector register.
            VRedSum | VRedAnd | VRedOr | VRedXor |
            VRedMinU | VRedMin | VRedMaxU | VRedMax |
            VFRedOSum | VFRedUSum | VFRedMax | VFRedMin |
            // Carry/borrow input ops (read v0 mask + full group operands)
            VAdc | VSbc
            => VecOperandGroups { vd: lmul, vs2: lmul, vs1: vs1_base },

            // ── Reductions need special vd handling ──────────────────────
            // Destination is a single register (element 0), not an LMUL group.
            // Moved above into the standard group since the execute path still
            // reads the old vd[0] as the accumulator identity — keep vd=lmul
            // for rename dependency tracking (conservative but correct).

            // ── Widening integer (vd=2×LMUL, sources=LMUL) ──────────────
            VWAddU | VWAdd | VWSubU | VWSub |
            VWMulU | VWMul | VWMulSU |
            VWMaccU | VWMacc | VWMaccSU | VWMaccUS |
            VWRedSumU | VWRedSum |
            // FP widening
            VFWAdd | VFWSub | VFWMul |
            VFWMacc | VFWNMacc | VFWMSac | VFWNMSac |
            VFWRedOSum | VFWRedUSum
            => {
                let emul2 = (lmul * 2).min(8);
                VecOperandGroups { vd: emul2, vs2: lmul, vs1: vs1_base }
            }

            // ── Widening with wide source (.wv/.wf: vd=2×LMUL, vs2=2×LMUL, vs1=LMUL)
            VWAddUW | VWAddW | VWSubUW | VWSubW |
            VFWAddW | VFWSubW
            => {
                let emul2 = (lmul * 2).min(8);
                VecOperandGroups { vd: emul2, vs2: emul2, vs1: vs1_base }
            }

            // ── Narrowing (vd=LMUL, vs2=2×LMUL) ────────────────────────
            VNSrl | VNSra | VNClipU | VNClip
            => {
                let emul2 = (lmul * 2).min(8);
                VecOperandGroups { vd: lmul, vs2: emul2, vs1: vs1_base }
            }

            // ── Zero/sign extension (vd=LMUL, vs2=LMUL/factor) ─────────
            // These read narrower source elements; vs2 group is smaller.
            // However, in practice EMUL for vs2 = LMUL/factor, which may be
            // fractional (< 1 register). Use 1 as minimum.
            VZextVf2 | VSextVf2 => VecOperandGroups { vd: lmul, vs2: (lmul / 2).max(1), vs1: 0 },
            VZextVf4 | VSextVf4 => VecOperandGroups { vd: lmul, vs2: (lmul / 4).max(1), vs1: 0 },
            VZextVf8 | VSextVf8 => VecOperandGroups { vd: lmul, vs2: (lmul / 8).max(1), vs1: 0 },

            // ── FP conversions (unary: vd=LMUL, vs2=LMUL, vs1=sub-opcode) ──
            VFCvtXuF | VFCvtXF | VFCvtFXu | VFCvtFX |
            VFCvtRtzXuF | VFCvtRtzXF
            => VecOperandGroups { vd: lmul, vs2: lmul, vs1: 0 },

            // ── Widening FP conversions (vd=2×LMUL, vs2=LMUL) ──────────
            VFWCvtXuF | VFWCvtXF | VFWCvtFXu | VFWCvtFX |
            VFWCvtFF | VFWCvtRtzXuF | VFWCvtRtzXF
            => {
                let emul2 = (lmul * 2).min(8);
                VecOperandGroups { vd: emul2, vs2: lmul, vs1: 0 }
            }

            // ── Narrowing FP conversions (vd=LMUL, vs2=2×LMUL) ─────────
            VFNCvtXuF | VFNCvtXF | VFNCvtFXu | VFNCvtFX |
            VFNCvtFF | VFNCvtRodFF | VFNCvtRtzXuF | VFNCvtRtzXF
            => {
                let emul2 = (lmul * 2).min(8);
                VecOperandGroups { vd: lmul, vs2: emul2, vs1: 0 }
            }

            // ── FP unary (sqrt, rsqrt, rec, class): vs1=sub-opcode ──────
            VFSqrt | VFRsqrt7 | VFRec7 | VFClass
            => VecOperandGroups { vd: lmul, vs2: lmul, vs1: 0 },

            // ── Scalar-result ops (vd is a GPR/FPR, not a vreg) ─────────
            // vmv.x.s reads vs2[0] only — but rename tracks full group
            // dependency conservatively (correct, slightly pessimistic).
            VMvXS => VecOperandGroups { vd: 0, vs2: lmul, vs1: 0 },
            VFMvFS => VecOperandGroups { vd: 0, vs2: lmul, vs1: 0 },
            VCPopM | VFirstM => VecOperandGroups { vd: 0, vs2: 1, vs1: 0 },

            // ── Scalar-to-vector (vmv.s.x, vfmv.s.f) ───────────────────
            // Writes only element 0 of vd. Still an LMUL group for rename
            // (old vd is needed for tail elements).
            VMvSX => VecOperandGroups { vd: lmul, vs2: 0, vs1: 0 },
            VFMvSF => VecOperandGroups { vd: lmul, vs2: 0, vs1: 0 },

            // ── Mask-producing comparisons (vd = single mask register) ──
            VMSeq | VMSne | VMSltu | VMSlt | VMSleu | VMSle | VMSgtu | VMSgt |
            VMFEq | VMFNe | VMFLt | VMFLe | VMFGt | VMFGe
            => VecOperandGroups { vd: 1, vs2: lmul, vs1: vs1_base },

            // ── Carry/borrow flag output (vd = single mask register) ────
            VMadc | VMsbc
            => VecOperandGroups { vd: 1, vs2: lmul, vs1: vs1_base },

            // ── Mask-source unary (vs2 = single mask reg, vd varies) ────
            VMSbfM | VMSofM | VMSifM
            => VecOperandGroups { vd: 1, vs2: 1, vs1: 0 },

            VIotaM => VecOperandGroups { vd: lmul, vs2: 1, vs1: 0 },

            // ── Mask logical (all operands are single mask registers) ────
            VMAndMM | VMNandMM | VMAndnMM | VMOrMM | VMNorMM | VMOrnMM |
            VMXorMM | VMXnorMM
            => VecOperandGroups { vd: 1, vs2: 1, vs1: 1 },

            // ── Whole-register moves (group size from opcode, not LMUL) ─
            VMv1r => VecOperandGroups { vd: 1, vs2: 1, vs1: 0 },
            VMv2r => VecOperandGroups { vd: 2, vs2: 2, vs1: 0 },
            VMv4r => VecOperandGroups { vd: 4, vs2: 4, vs1: 0 },
            VMv8r => VecOperandGroups { vd: 8, vs2: 8, vs1: 0 },

            // ── Whole-register loads/stores (group from nf, not LMUL) ───
            // These are handled separately in decode via vec_nf; here we
            // report LMUL as the conservative group.
            VLoadWholeReg | VStoreWholeReg
            => VecOperandGroups { vd: lmul, vs2: 0, vs1: 0 },

            // ── Memory ops (vd/vs3 = LMUL group, vs2 = index vector) ───
            VLoadUnit | VLoadFF | VStoreUnit |
            VLoadStride | VStoreStride
            => VecOperandGroups { vd: lmul, vs2: 0, vs1: 0 },

            VLoadIndexOrd | VLoadIndexUnord
            => VecOperandGroups { vd: lmul, vs2: lmul, vs1: 0 },

            VStoreIndexOrd | VStoreIndexUnord
            => VecOperandGroups { vd: lmul, vs2: lmul, vs1: 0 },

            // ── Mask load/store ──────────────────────────────────────────
            VLoadMask | VStoreMask
            => VecOperandGroups { vd: 1, vs2: 0, vs1: 0 },
        }
    }
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
    /// Vector-immediate (OPIVI).
    VI,
    /// Vector-scalar FP (OPFVF).
    VF,
}
