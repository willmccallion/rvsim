//! The control signals an instruction decodes to.
//!
//! Which operation it performs, where its operands come from, how it
//! accesses memory and what system action it takes.

use crate::common::CsrAddr;
use crate::core::units::fpu::rounding_modes::RoundingMode;
use crate::isa::op::{AluOp, AtomicOp, CsrOp, MemWidth, SystemOp, VecSrcEncoding, VectorOp};
use crate::isa::rvv::{Sew, VRegIdx};

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
    /// Instruction operates on 16-bit half-precision floats (Zfh).
    /// When set, `is_rv32` is ignored by the FP units.
    pub is_f16: bool,
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
    /// Floating-point rounding mode for FP arithmetic and conversions.
    /// `None` means use `fcsr.frm` (dynamic). Set during decode from funct3.
    pub fp_rm: Option<RoundingMode>,
    /// `rs1` is a floating-point register.
    pub rs1_fp: bool,
    /// `rs2` is a floating-point register.
    pub rs2_fp: bool,
    /// `rs3` is a floating-point register.
    pub rs3_fp: bool,
    /// Atomic memory operation type.
    pub atomic_op: AtomicOp,
    /// An atomic with the `aq` bit: no younger load may perform before it.
    pub acquire: bool,
    /// An atomic with the `rl` bit: every older access, stores included,
    /// must be performed before it is.
    pub release: bool,
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
    /// `true` if the source LMUL is fractional (Mf2/Mf4/Mf8). Distinguishes
    /// from M1, which `vec_lmul_regs` collapses to the same value of `1`.
    /// Needed by `operand_groups` to compute the right widening group size.
    pub vec_lmul_is_fractional: bool,
    /// `true` for `.vs`-form crypto ops where vs2 element group 0 is
    /// broadcast across all destination element groups (vaesem/ef/dm/df.vs,
    /// vaesz.vs, vsm4r.vs). Distinguishes from the `.vv` form which uses a
    /// per-group key. Set at decode based on funct6 (0x29 vs 0x28).
    pub vec_broadcast_vs2: bool,
}

impl ControlSignals {
    /// True for anything that reads memory: a scalar load, LR, an AMO, or a
    /// vector load, which decodes without `mem_read`.
    pub const fn reads_memory(&self) -> bool {
        self.mem_read || crate::core::units::vpu::mem::is_vec_load(self.vec_op)
    }

    /// True for anything that writes memory: a scalar store, SC, an AMO, or
    /// a vector store, which decodes without `mem_write`.
    pub const fn writes_memory(&self) -> bool {
        self.mem_write || crate::core::units::vpu::mem::is_vec_store(self.vec_op)
    }

    /// True for an atomic that executes only as the oldest instruction,
    /// once every older store has been written, and takes effect in the
    /// cache: an AMO or SC (gem5's non-speculative atomics), or an LR with
    /// `rl`, which must follow every older store.
    #[must_use]
    pub const fn performs_at_rob_head(&self) -> bool {
        match self.atomic_op {
            AtomicOp::None => false,
            AtomicOp::Lr => self.release,
            _ => true,
        }
    }

    /// True for an instruction that takes a store-buffer slot: a store, an
    /// SC or AMO, or a cache-block operation, which is ordered as a store.
    #[must_use]
    pub const fn uses_store_buffer(&self) -> bool {
        self.mem_write || self.system_op.is_cbo()
    }

    /// True for a scalar instruction memory1 must translate: a load, a
    /// store, an atomic or a cache-block operation. It completes after the
    /// memory stages rather than when its unit finishes.
    pub const fn uses_memory_pipeline(&self) -> bool {
        self.mem_read
            || self.mem_write
            || !matches!(self.atomic_op, AtomicOp::None)
            || self.system_op.is_cbo()
    }
}
