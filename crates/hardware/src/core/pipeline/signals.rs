//! Pipeline control signals and operation types.
//!
//! This module defines the signals that control instruction execution. It performs:
//! 1. **Operation Classification:** Categorizes ALU, atomic, and CSR operations.
//! 2. **Operand Selection:** Defines sources for ALU inputs (registers, PC, or immediates).
//! 3. **Memory Control:** Specifies access widths and sign-extension requirements.
//! 4. **System Control:** Manages privilege transitions and system-level instructions.

use crate::common::CsrAddr;
use crate::core::units::vpu::types::VRegIdx;

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
}

/// Vector operation type. Initially just configuration ops for Phase 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VectorOp {
    /// No vector operation.
    #[default]
    None,
    /// vsetvli — set vl/vtype from rs1 and immediate.
    Vsetvli,
    /// vsetivli — set vl/vtype from uimm and immediate.
    Vsetivli,
    /// vsetvl — set vl/vtype from rs1 and rs2.
    Vsetvl,
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
