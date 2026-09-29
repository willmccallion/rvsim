//! Pipeline control signals and operation types.
//!
//! This module defines the signals that control instruction execution. It performs:
//! 1. **Operation Classification:** Categorizes ALU, atomic, and CSR operations.
//! 2. **Operand Selection:** Defines sources for ALU inputs (registers, PC, or immediates).
//! 3. **Memory Control:** Specifies access widths and sign-extension requirements.
//! 4. **System Control:** Manages privilege transitions and system-level instructions.

use crate::common::CsrAddr;
use crate::core::units::fpu::rounding_modes::RoundingMode;
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

    /// Convert half-precision to single-precision float (Zfh).
    FCvtSH,

    /// Convert single-precision to half-precision float (Zfh).
    FCvtHS,

    /// Convert half-precision to double-precision float (Zfh).
    FCvtDH,

    /// Convert double-precision to half-precision float (Zfh).
    FCvtHD,

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

    /// Shift-left-1 and add (sh1add).
    Sh1Add,

    /// Shift-left-2 and add (sh2add).
    Sh2Add,

    /// Shift-left-3 and add (sh3add).
    Sh3Add,

    /// Add unsigned word (add.uw) — zero-extends rs1[31:0] before adding.
    AddUw,

    /// Shift-left-1 and add unsigned word (sh1add.uw).
    Sh1AddUw,

    /// Shift-left-2 and add unsigned word (sh2add.uw).
    Sh2AddUw,

    /// Shift-left-3 and add unsigned word (sh3add.uw).
    Sh3AddUw,

    /// Shift-left-logical unsigned word immediate (slli.uw).
    SlliUw,

    /// Bitwise AND with complement (andn).
    Andn,

    /// Bitwise OR with complement (orn).
    Orn,

    /// Bitwise exclusive NOR (xnor).
    Xnor,

    /// Count leading zeros (clz / clzw).
    Clz,

    /// Count trailing zeros (ctz / ctzw).
    Ctz,

    /// Count set bits / population count (cpop / cpopw).
    Cpop,

    /// Maximum (signed).
    Max,

    /// Maximum (unsigned).
    Maxu,

    /// Minimum (signed).
    Min,

    /// Minimum (unsigned).
    Minu,

    /// Sign-extend byte.
    SextB,

    /// Sign-extend halfword.
    SextH,

    /// Rotate left (rol / rolw).
    Rol,

    /// Rotate right (ror / rorw / rori / roriw).
    Ror,

    /// OR-combine bytes (orc.b).
    OrcB,

    /// Byte-reverse (rev8).
    Rev8,

    /// Carry-less multiply (low half).
    Clmul,

    /// Carry-less multiply (high half).
    Clmulh,

    /// Carry-less multiply (reversed / remainder).
    Clmulr,

    /// Clear single bit (bclr / bclri).
    Bclr,

    /// Extract single bit (bext / bexti).
    Bext,

    /// Invert single bit (binv / binvi).
    Binv,

    /// Set single bit (bset / bseti).
    Bset,

    /// Bit-reverse within each byte (brev8).
    Brev8,

    /// Pack lower halves of two registers (pack).
    Pack,

    /// Pack lowest bytes of two registers (packh).
    Packh,

    /// Pack lower halves, 32-bit variant (packw).
    Packw,

    /// 4-bit crossbar permutation (xperm4).
    Xperm4,

    /// 8-bit crossbar permutation (xperm8).
    Xperm8,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

impl MemWidth {
    /// Bytes moved by an access of this width; zero for `Nop`.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Nop => 0,
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
            Self::Double => 8,
        }
    }
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

    /// `CBO.ZERO` (Zicboz) — zero a cache-block-aligned region at rs1.
    CboZero,

    /// `CBO.INVAL` (Zicbom) — invalidate the L1D line at rs1.
    CboInval,

    /// `CBO.CLEAN` (Zicbom) — writeback the L1D line at rs1, keep it valid.
    CboClean,

    /// `CBO.FLUSH` (Zicbom) — writeback then invalidate the L1D line at rs1.
    CboFlush,

    /// `ECALL` — environment call.
    Ecall,

    /// `CSRRW`/`CSRRS`/`CSRRC` and their immediate forms; `csr_op` says which.
    Csr,
}

impl SystemOp {
    /// True for the instructions gem5 marks `IsSerializeAfter`: an
    /// out-of-order core renames nothing younger until they commit.
    pub const fn serializes_after(self) -> bool {
        matches!(
            self,
            Self::Csr
                | Self::Ecall
                | Self::Mret
                | Self::Sret
                | Self::Wfi
                | Self::SfenceVma
                | Self::FenceI
        )
    }

    /// True for the Zicboz/Zicbom cache-block operations.
    pub const fn is_cbo(self) -> bool {
        matches!(self, Self::CboZero | Self::CboInval | Self::CboClean | Self::CboFlush)
    }
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

/// Vector operation type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VectorOp {
    /// No vector operation.
    #[default]
    None,

    /// `vsetvli` — set vl/vtype from rs1 and immediate.
    Vsetvli,
    /// `vsetivli` — set vl/vtype from uimm and immediate.
    Vsetivli,
    /// `vsetvl` — set vl/vtype from rs1 and rs2.
    Vsetvl,

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

    /// `vmerge` / `vmv` — vector merge or move.
    VMerge,

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

    /// `vadc` — add with carry from v0 mask.
    VAdc,
    /// `vmadc` — mask-producing add with carry.
    VMadc,
    /// `vsbc` — subtract with borrow from v0 mask.
    VSbc,
    /// `vmsbc` — mask-producing subtract with borrow.
    VMsbc,

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

    /// `vdivu` — unsigned divide.
    VDivU,
    /// `vdiv` — signed divide.
    VDiv,
    /// `vremu` — unsigned remainder.
    VRemU,
    /// `vrem` — signed remainder.
    VRem,

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

    /// `vnsrl` — narrowing shift right logical (2×SEW → SEW).
    VNSrl,
    /// `vnsra` — narrowing shift right arithmetic (2×SEW → SEW).
    VNSra,
    /// `vnclipu` — narrowing clip unsigned with saturation.
    VNClipU,
    /// `vnclip` — narrowing clip signed with saturation.
    VNClip,

    /// `vsaddu` — saturating unsigned add.
    VSAddU,
    /// `vsadd` — saturating signed add.
    VSAdd,
    /// `vssubu` — saturating unsigned subtract.
    VSSubU,
    /// `vssub` — saturating signed subtract.
    VSSub,

    /// `vaaddu` — averaging unsigned add.
    VAAddU,
    /// `vaadd` — averaging signed add.
    VAAdd,
    /// `vasubu` — averaging unsigned subtract.
    VASubU,
    /// `vasub` — averaging signed subtract.
    VASub,

    /// `vsmul` — signed fractional multiply with rounding.
    VSmul,
    /// `vssrl` — scaling shift right logical with rounding.
    VSSrl,
    /// `vssra` — scaling shift right arithmetic with rounding.
    VSSra,

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

    /// `vandn` — vector bitwise AND-NOT (vd[i] = vs2[i] & ~op1[i]).
    VAndN,
    /// `vbrev.v` — reverse all bits within each element.
    VBrev,
    /// `vbrev8.v` — reverse bits within each byte of each element.
    VBrev8,
    /// `vrev8.v` — reverse bytes within each element.
    VRev8,
    /// `vclz.v` — count leading zeros within each element (at SEW).
    VClz,
    /// `vctz.v` — count trailing zeros within each element (at SEW).
    VCtz,
    /// `vcpop.v` — per-element population count (Zvbb; distinct from `vcpop.m`).
    VCpopV,
    /// `vrol` — vector rotate left (vv/vx).
    VRol,
    /// `vror` — vector rotate right (vv/vx/vi with 6-bit imm).
    VRor,
    /// `vwsll` — widening shift left logical (vd is 2*SEW).
    VWsll,

    /// `vclmul` — carry-less multiply low (GF(2) multiply, low half).
    VClMul,
    /// `vclmulh` — carry-less multiply high (GF(2) multiply, high half).
    VClMulH,

    /// `vaesem.vv`/`vaesem.vs` — AES single-round encryption (middle round).
    VAesEm,
    /// `vaesef.vv`/`vaesef.vs` — AES single-round encryption (final round).
    VAesEf,
    /// `vaesdm.vv`/`vaesdm.vs` — AES single-round decryption (middle round).
    VAesDm,
    /// `vaesdf.vv`/`vaesdf.vs` — AES single-round decryption (final round).
    VAesDf,
    /// `vaesz.vs` — AES round-zero (XOR with key).
    VAesZ,
    /// `vaeskf1.vi` — AES-128 forward key schedule.
    VAesKf1,
    /// `vaeskf2.vi` — AES-256 forward key schedule.
    VAesKf2,

    /// `vsha2ms.vv` — SHA-2 message scheduling.
    VSha2Ms,
    /// `vsha2ch.vv` — SHA-2 compression (high half).
    VSha2Ch,
    /// `vsha2cl.vv` — SHA-2 compression (low half).
    VSha2Cl,

    /// `vsm3me.vv` — SM3 message expansion.
    VSm3Me,
    /// `vsm3c.vi` — SM3 compression.
    VSm3C,

    /// `vsm4r.vv`/`vsm4r.vs` — SM4 round.
    VSm4R,
    /// `vsm4k.vi` — SM4 key expansion.
    VSm4K,

    /// `vghsh.vv` — vector GHASH add-multiply.
    VGhsh,
    /// `vgmul.vv` — vector GHASH multiply.
    VGmul,

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

    /// Strided vector load (`vlse8/16/32/64`).
    VLoadStride,
    /// Strided vector store (`vsse8/16/32/64`).
    VStoreStride,

    /// Indexed ordered vector load (`vloxei8/16/32/64`).
    VLoadIndexOrd,
    /// Indexed ordered vector store (`vsoxei8/16/32/64`).
    VStoreIndexOrd,
    /// Indexed unordered vector load (`vluxei8/16/32/64`).
    VLoadIndexUnord,
    /// Indexed unordered vector store (`vsuxei8/16/32/64`).
    VStoreIndexUnord,

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

    /// `vfmin` — vector FP minimum.
    VFMin,
    /// `vfmax` — vector FP maximum.
    VFMax,

    /// `vfsgnj` — vector FP sign injection (copy sign).
    VFSgnj,
    /// `vfsgnjn` — vector FP negated sign injection.
    VFSgnjn,
    /// `vfsgnjx` — vector FP XOR sign injection.
    VFSgnjx,

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

    /// `vfsqrt` — vector FP square root.
    VFSqrt,
    /// `vfrsqrt7` — vector FP reciprocal square root (7-bit accuracy).
    VFRsqrt7,
    /// `vfrec7` — vector FP reciprocal (7-bit accuracy).
    VFRec7,
    /// `vfclass` — vector FP classify.
    VFClass,

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

    /// `vfwmacc` — widening FP multiply-accumulate.
    VFWMacc,
    /// `vfwnmacc` — widening FP negated multiply-accumulate.
    VFWNMacc,
    /// `vfwmsac` — widening FP multiply-subtract accumulate.
    VFWMSac,
    /// `vfwnmsac` — widening FP negated multiply-subtract accumulate.
    VFWNMSac,

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

    /// `vfmerge` — vector FP merge with mask.
    VFMerge,
    /// `vfmv.s.f` — move FP scalar to vector element 0.
    VFMvSF,
    /// `vfmv.f.s` — move vector element 0 to FP scalar.
    VFMvFS,

    /// `vfslide1up` — slide up by one with FP scalar.
    VFSlide1Up,
    /// `vfslide1down` — slide down by one with FP scalar.
    VFSlide1Down,

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

    /// `vwredsumu` — widening unsigned reduction sum.
    VWRedSumU,
    /// `vwredsum` — widening signed reduction sum.
    VWRedSum,

    /// `vfredosum` — FP ordered reduction sum.
    VFRedOSum,
    /// `vfredusum` — FP unordered reduction sum.
    VFRedUSum,
    /// `vfredmax` — FP reduction maximum.
    VFRedMax,
    /// `vfredmin` — FP reduction minimum.
    VFRedMin,

    /// `vfwredosum` — widening FP ordered reduction sum.
    VFWRedOSum,
    /// `vfwredusum` — widening FP unordered reduction sum.
    VFWRedUSum,

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

    /// `vcpop.m` — count population of mask register.
    VCPopM,
    /// `vfirst.m` — find first set bit in mask register.
    VFirstM,

    /// `vmsbf.m` — set-before-first mask bit.
    VMSbfM,
    /// `vmsif.m` — set-including-first mask bit.
    VMSifM,
    /// `vmsof.m` — set-only-first mask bit.
    VMSofM,

    /// `viota.m` — iota (prefix sum of mask bits).
    VIotaM,
    /// `vid.v` — vector element index.
    VIdV,

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
    /// `vsetvli`, `vsetivli` or `vsetvl`: writes `vtype` and `vl` instead of
    /// a vector register.
    #[must_use]
    pub const fn is_config(self) -> bool {
        matches!(self, Self::Vsetvli | Self::Vsetivli | Self::Vsetvl)
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
    #[allow(clippy::enum_glob_use)]
    pub fn operand_groups(
        self,
        lmul: u8,
        lmul_is_fractional: bool,
        src_enc: VecSrcEncoding,
        nf: u8,
        broadcast_vs2: bool,
    ) -> VecOperandGroups {
        use VectorOp::*;

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
            None | Vsetvli | Vsetivli | Vsetvl => VecOperandGroups { vd: 0, vs1: 0, vs2: 0 },

            VAdd | VSub | VRsub | VAnd | VOr | VXor | VSll | VSrl | VSra | VMinU | VMin | VMaxU
            | VMax | VMul | VMulh | VMulhu | VMulhsu | VMacc | VNMSac | VMadd | VNMSub | VDivU
            | VDiv | VRemU | VRem | VSAddU | VSAdd | VSSubU | VSSub | VAAddU | VAAdd | VASubU
            | VASub | VSmul | VSSrl | VSSra | VMerge | VSlideUp | VSlideDown | VSlide1Up
            | VSlide1Down | VRgather | VRgatherEi16 | VCompress | VFAdd | VFSub | VFRSub
            | VFMul | VFDiv | VFRDiv | VFMin | VFMax | VFSgnj | VFSgnjn | VFSgnjx | VFMacc
            | VFNMacc | VFMSac | VFNMSac | VFMAdd | VFNMAdd | VFMSub | VFNMSub | VFMerge
            | VFSlide1Up | VFSlide1Down | VAdc | VSbc | VAndN | VRol | VRor | VClMul | VClMulH => {
                VecOperandGroups { vd: lmul, vs2: lmul, vs1: vs1_base }
            }

            // Zvknh / Zvksh vsm3me / Zvkg vghsh: vs1 is a per-group vector input.
            VSha2Ms | VSha2Ch | VSha2Cl | VSm3Me | VGhsh => {
                VecOperandGroups { vd: lmul, vs2: lmul, vs1: lmul }
            }

            // AES/SM4 rounds: vs1 field is a sub-opcode, not a register; .vs broadcasts vs2 EMUL=1.
            VAesEm | VAesEf | VAesDm | VAesDf | VAesZ | VSm4R => {
                VecOperandGroups { vd: lmul, vs2: vs2_crypto, vs1: 0 }
            }

            // vs1 field is a sub-opcode/imm here, not a register reference.
            VAesKf1 | VAesKf2 | VSm4K | VSm3C | VGmul | VFCvtXuF | VFCvtXF | VFCvtFXu | VFCvtFX
            | VFCvtRtzXuF | VFCvtRtzXF | VFSqrt | VFRsqrt7 | VFRec7 | VFClass | VLoadIndexOrd
            | VLoadIndexUnord | VStoreIndexOrd | VStoreIndexUnord | VBrev | VBrev8 | VRev8
            | VClz | VCtz | VCpopV => VecOperandGroups { vd: lmul, vs2: lmul, vs1: 0 },

            // RVV 1.0 §14.1: reduction vd/vs1 are single registers; vs2 is the full LMUL group.
            VRedSum | VRedAnd | VRedOr | VRedXor | VRedMinU | VRedMin | VRedMaxU | VRedMax
            | VFRedOSum | VFRedUSum | VFRedMax | VFRedMin | VWRedSumU | VWRedSum | VFWRedOSum
            | VFWRedUSum => VecOperandGroups { vd: 1, vs2: lmul, vs1: 1 },

            VWAddU | VWAdd | VWSubU | VWSub | VWMulU | VWMul | VWMulSU | VWMaccU | VWMacc
            | VWMaccSU | VWMaccUS | VWsll | VFWAdd | VFWSub | VFWMul | VFWMacc | VFWNMacc
            | VFWMSac | VFWNMSac => VecOperandGroups { vd: emul_widened, vs2: lmul, vs1: vs1_base },

            VWAddUW | VWAddW | VWSubUW | VWSubW | VFWAddW | VFWSubW => {
                VecOperandGroups { vd: emul_widened, vs2: emul_widened, vs1: vs1_base }
            }

            VNSrl | VNSra | VNClipU | VNClip => {
                VecOperandGroups { vd: lmul, vs2: emul_widened, vs1: vs1_base }
            }

            // Use 1 as minimum; vs2 EMUL = LMUL/factor may be fractional (<1 register).
            VZextVf2 | VSextVf2 => VecOperandGroups { vd: lmul, vs2: (lmul / 2).max(1), vs1: 0 },
            VZextVf4 | VSextVf4 => VecOperandGroups { vd: lmul, vs2: (lmul / 4).max(1), vs1: 0 },
            VZextVf8 | VSextVf8 => VecOperandGroups { vd: lmul, vs2: (lmul / 8).max(1), vs1: 0 },

            VFWCvtXuF | VFWCvtXF | VFWCvtFXu | VFWCvtFX | VFWCvtFF | VFWCvtRtzXuF | VFWCvtRtzXF => {
                VecOperandGroups { vd: emul_widened, vs2: lmul, vs1: 0 }
            }

            VFNCvtXuF | VFNCvtXF | VFNCvtFXu | VFNCvtFX | VFNCvtFF | VFNCvtRodFF | VFNCvtRtzXuF
            | VFNCvtRtzXF => VecOperandGroups { vd: lmul, vs2: emul_widened, vs1: 0 },

            // RVV 1.0 §16.1: vmv.x.s / vfmv.f.s read only element 0 of vs2.
            VMvXS | VFMvFS | VCPopM | VFirstM => VecOperandGroups { vd: 0, vs2: 1, vs1: 0 },

            // vmv.s.x / vfmv.s.f write only element 0 (§16.1); vlm/vsm: single mask register.
            VMvSX | VFMvSF | VLoadMask | VStoreMask => VecOperandGroups { vd: 1, vs2: 0, vs1: 0 },

            VMSeq | VMSne | VMSltu | VMSlt | VMSleu | VMSle | VMSgtu | VMSgt | VMFEq | VMFNe
            | VMFLt | VMFLe | VMFGt | VMFGe | VMadc | VMsbc => {
                VecOperandGroups { vd: 1, vs2: lmul, vs1: vs1_base }
            }

            VMSbfM | VMSofM | VMSifM | VMv1r => VecOperandGroups { vd: 1, vs2: 1, vs1: 0 },

            VIotaM => VecOperandGroups { vd: lmul, vs2: 1, vs1: 0 },

            // vid.v's vs2 field is part of the opcode (must be zero), not a register.
            VIdV | VLoadUnit | VLoadFF | VStoreUnit | VLoadStride | VStoreStride => {
                VecOperandGroups { vd: lmul, vs2: 0, vs1: 0 }
            }

            VMAndMM | VMNandMM | VMAndnMM | VMOrMM | VMNorMM | VMOrnMM | VMXorMM | VMXnorMM => {
                VecOperandGroups { vd: 1, vs2: 1, vs1: 1 }
            }

            VMv2r => VecOperandGroups { vd: 2, vs2: 2, vs1: 0 },
            VMv4r => VecOperandGroups { vd: 4, vs2: 4, vs1: 0 },
            VMv8r => VecOperandGroups { vd: 8, vs2: 8, vs1: 0 },

            // nf encoding: 0=1reg, 1=2reg, 3=4reg, 7=8reg; group = nf+1.
            VLoadWholeReg | VStoreWholeReg => {
                let regs = nf + 1;
                VecOperandGroups { vd: regs, vs2: 0, vs1: 0 }
            }
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
