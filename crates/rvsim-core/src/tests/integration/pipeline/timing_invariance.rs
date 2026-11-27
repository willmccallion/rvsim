//! Timing properties of branch-free, memory-free programs.
//!
//! No scalar or vector unit's latency depends on the values it computes, so
//! such a program takes as many cycles whatever its immediates hold, and
//! whatever registers it names as long as it renames them consistently. An
//! instruction that waited on a register it does not read breaks both.
//!
//! A program's vector instructions run under the one `vtype` and `vl` a
//! `vsetivli` at its start sets. They are unmasked, so `v0` is an ordinary
//! register, and their register groups are renamed whole, so every group
//! stays aligned to LMUL.

use proptest::collection::vec;
use proptest::prelude::*;
use proptest::sample::select;

use crate::config::{BackendKind, Config};
use crate::isa::encode::{IType, R4Type, RType, UType};
use crate::isa::encoding::rv64bk::funct3::{CLZ_IMM, CPOP_IMM};
use crate::isa::encoding::rv64d::funct7::{
    FADD_D, FCLASS_MV_X_D, FCVT_D_W, FCVT_W_D, FDIV_D, FMUL_D, FMV_D_X, FSQRT_D,
};
use crate::isa::encoding::rv64d::opcodes::{OP_FMADD, OP_FP};
use crate::isa::encoding::rv64i::funct3::{ADD_SUB, SLL, SLT, SRL_SRA, XOR};
use crate::isa::encoding::rv64i::funct7::{DEFAULT, SUB};
use crate::isa::encoding::rv64i::opcodes::{
    OP_AUIPC, OP_IMM, OP_IMM_32, OP_LUI, OP_REG, OP_REG_32,
};
use crate::isa::encoding::rv64m::funct3::{DIV, MUL, MULH, REM};
use crate::isa::encoding::rv64m::opcodes::M_EXTENSION;
use crate::isa::encoding::rvv::funct3::{OPCFG, OPFVF, OPFVV, OPIVI, OPIVV, OPIVX, OPMVV, OPMVX};
use crate::isa::encoding::rvv::funct6::{
    VADD, VAES_KF1, VAES_KF2, VAES_VS1_DM, VAES_VS1_EF, VCRYPTO_VV, VDIV, VFADD, VFDIV, VFMACC,
    VFMUL, VFREDOSUM, VFREDUSUM, VFUNARY0, VFUNARY1, VGHSH, VGMUL_VS1, VMACC, VMAND, VMFLT, VMSEQ,
    VMSLE, VMSLTU, VMUL, VMULH, VMUNARY0, VMUNARY0_VID_V, VREDMAX, VREDSUM, VREM, VRFUNARY0,
    VRFUNARY0_VFMV_S_F, VRGATHER, VRXUNARY0, VSHA2_CH, VSHA2_CL, VSHA2_MS, VSLIDEDOWN, VSLIDEUP,
    VSLL, VSM4_K, VSM4_VS1_R, VSRA, VSUB, VWFUNARY0, VWFUNARY0_VFMV_F_S, VWXUNARY0,
    VWXUNARY0_VCPOP_M, VWXUNARY0_VFIRST_M, VWXUNARY0_VMV_X_S, VXOR, VXUNARY0, VXUNARY0_VSEXT_VF4,
    VXUNARY0_VZEXT_VF2,
};
use crate::isa::encoding::rvv::opcodes::{OP_V, OP_V_CRYPTO};
use crate::isa::reg::RegIdx;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::X0;
use crate::tests::support::seeded_cases;

const CASES: u32 = 512;
const MAX_INSTRUCTIONS: usize = 24;
const CYCLE_LIMIT: u64 = 20_000;
const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];
const VECTOR_REGISTERS: u8 = 32;

/// The `fcvt` rs2 code for a 64-bit signed integer.
const FCVT_LONG: u8 = 2;
/// The funct3 of every FP operation here: round to nearest, ties to even,
/// for those that round, and the value `fmv` requires.
const FP_FUNCT3: u32 = 0;
/// The `fmt` field of a double-precision fused multiply-add.
const FMT_DOUBLE: u32 = 1;
/// `srai`'s funct6, which sits in its immediate above the shift amount.
const SRAI_HIGH_BITS: i32 = 0x400;
/// The vs1 code of `vfsqrt.v` under `VFUNARY1`.
const VFSQRT_V: u8 = 0b00000;
/// The vs1 code of `vfcvt.x.f.v` under `VFUNARY0`.
const VFCVT_X_F_V: u8 = 0b00001;
/// The vs2 code of `vmv.s.x` under `VRXUNARY0`.
const VMV_S_X: u8 = 0b00000;
/// `vid.v`'s vs2 field, which names no source and must be zero.
const VID_V_VS2: u8 = 0b00000;
/// The width of an OP-V immediate.
const IMM5_MASK: i32 = 0x1f;
/// An OP-V instruction's `vm` bit, set: unmasked.
const UNMASKED: u32 = 1 << 25;
/// The `vl` a `vsetivli` asks for: a multiple of four, the element group
/// every crypto operation here works in, so the `vl` it gets, the lesser of
/// this and VLMAX, is one too for 32-bit elements.
const AVL: u32 = 28;
/// The `vsew` field for 32-bit elements, the only width the crypto
/// operations take.
const E32: u32 = 0b010;
/// The `vsew` field for 64-bit elements.
const E64: u32 = 0b011;

/// A register file an encoding's field names a register in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum File {
    Int,
    Fp,
    Vec,
}

/// What an OP-V instruction's vs1 or vs2 field holds.
#[derive(Clone, Copy, Debug)]
enum Field {
    /// A register of this file.
    Reg(File),
    /// A five-bit immediate.
    Imm,
    /// A fixed sub-opcode.
    Code(u8),
}

use Field::{Code, Imm, Reg};

const V: Field = Reg(File::Vec);
const X: Field = Reg(File::Int);
const F: Field = Reg(File::Fp);

/// Which sources an OP-V instruction's destination group may not overlap,
/// the encoding being reserved if it does.
#[derive(Clone, Copy, Debug)]
enum Overlap {
    Allowed,
    NotVs2,
    NotSources,
}

/// The operations the programs draw from: every scalar unit, the vector
/// units, and encodings whose rs1, rs2, vs1 or vs2 field holds an immediate,
/// a scalar register or a sub-opcode.
#[derive(Clone, Copy, Debug)]
enum Op {
    /// OP or OP-32: `rd`, `rs1` and `rs2` are integer registers.
    IntReg { opcode: u32, funct3: u32, funct7: u32 },
    /// OP-IMM or OP-IMM-32 with a 12-bit immediate.
    IntImm { opcode: u32, funct3: u32 },
    /// A shift by an immediate amount no wider than `shamt_mask`, under
    /// the funct6 bits `high`.
    Shift { opcode: u32, funct3: u32, high: i32, shamt_mask: i32 },
    /// An OP-IMM whose immediate field is a sub-opcode (`clz`, `cpop`).
    IntUnary { imm12: u32 },
    /// LUI or AUIPC, whose immediate covers the rs1 and rs2 fields.
    Upper { opcode: u32 },
    /// A double-precision OP-FP reading `rs1` and `rs2`.
    FpBinary { funct7: u32 },
    /// An OP-FP whose rs2 field is the sub-opcode `code`, reading `rs1`
    /// from `src` and writing `rd` in `dst`.
    FpUnary { funct7: u32, code: u8, src: File, dst: File },
    /// `fmadd.d`.
    Fma,
    /// An unmasked OP-V or OP-V-CRYPTO instruction writing a register of
    /// `vd`'s file.
    Vector {
        opcode: u32,
        funct3: u32,
        funct6: u32,
        vd: File,
        vs1: Field,
        vs2: Field,
        overlap: Overlap,
    },
}

use Op::{Fma, FpBinary, FpUnary, IntImm, IntReg, IntUnary, Shift, Upper, Vector};

/// An OP-V operation whose destination group may overlap its sources.
const fn vector(funct3: u32, funct6: u32, vd: File, vs1: Field, vs2: Field) -> Op {
    Vector { opcode: OP_V, funct3, funct6, vd, vs1, vs2, overlap: Overlap::Allowed }
}

/// A vector crypto operation, all of which are OPMVV and write a vector
/// register group.
const fn crypto(funct6: u32, vs1: Field, vs2: Field) -> Op {
    let funct3 = OPMVV;
    Vector {
        opcode: OP_V_CRYPTO,
        funct3,
        funct6,
        vd: File::Vec,
        vs1,
        vs2,
        overlap: Overlap::Allowed,
    }
}

impl Op {
    /// This vector operation with its destination group kept off the
    /// sources `overlap` names, as its encoding reserves.
    const fn reserving(self, overlap: Overlap) -> Self {
        let Vector { opcode, funct3, funct6, vd, vs1, vs2, .. } = self else {
            panic!("only a vector operation reserves an overlap")
        };
        Vector { opcode, funct3, funct6, vd, vs1, vs2, overlap }
    }

    /// Whether `shape`'s `vtype` leaves this operation's encoding defined.
    const fn is_defined_for(self, shape: VectorShape) -> bool {
        match self {
            Vector { opcode: OP_V_CRYPTO, .. } => shape.vsew == E32,
            _ => true,
        }
    }
}

const OPS: &[Op] = &[
    IntReg { opcode: OP_REG, funct3: ADD_SUB, funct7: DEFAULT },
    IntReg { opcode: OP_REG, funct3: ADD_SUB, funct7: SUB },
    IntReg { opcode: OP_REG, funct3: XOR, funct7: DEFAULT },
    IntReg { opcode: OP_REG, funct3: MUL, funct7: M_EXTENSION },
    IntReg { opcode: OP_REG, funct3: MULH, funct7: M_EXTENSION },
    IntReg { opcode: OP_REG, funct3: DIV, funct7: M_EXTENSION },
    IntReg { opcode: OP_REG, funct3: REM, funct7: M_EXTENSION },
    IntReg { opcode: OP_REG_32, funct3: ADD_SUB, funct7: DEFAULT },
    IntReg { opcode: OP_REG_32, funct3: MUL, funct7: M_EXTENSION },
    IntImm { opcode: OP_IMM, funct3: ADD_SUB },
    IntImm { opcode: OP_IMM, funct3: XOR },
    IntImm { opcode: OP_IMM, funct3: SLT },
    IntImm { opcode: OP_IMM_32, funct3: ADD_SUB },
    Shift { opcode: OP_IMM, funct3: SLL, high: 0, shamt_mask: 63 },
    Shift { opcode: OP_IMM, funct3: SRL_SRA, high: SRAI_HIGH_BITS, shamt_mask: 63 },
    Shift { opcode: OP_IMM_32, funct3: SLL, high: 0, shamt_mask: 31 },
    IntUnary { imm12: CLZ_IMM },
    IntUnary { imm12: CPOP_IMM },
    Upper { opcode: OP_LUI },
    Upper { opcode: OP_AUIPC },
    FpBinary { funct7: FADD_D },
    FpBinary { funct7: FMUL_D },
    FpBinary { funct7: FDIV_D },
    FpUnary { funct7: FSQRT_D, code: 0, src: File::Fp, dst: File::Fp },
    FpUnary { funct7: FCVT_D_W, code: FCVT_LONG, src: File::Int, dst: File::Fp },
    FpUnary { funct7: FCVT_W_D, code: FCVT_LONG, src: File::Fp, dst: File::Int },
    FpUnary { funct7: FMV_D_X, code: 0, src: File::Int, dst: File::Fp },
    FpUnary { funct7: FCLASS_MV_X_D, code: 0, src: File::Fp, dst: File::Int },
    Fma,
    vector(OPIVV, VADD, File::Vec, V, V),
    vector(OPIVX, VADD, File::Vec, X, V),
    vector(OPIVI, VADD, File::Vec, Imm, V),
    vector(OPIVX, VSUB, File::Vec, X, V),
    vector(OPIVV, VXOR, File::Vec, V, V),
    vector(OPIVI, VSLL, File::Vec, Imm, V),
    vector(OPIVX, VSRA, File::Vec, X, V),
    vector(OPIVV, VMSEQ, File::Vec, V, V),
    vector(OPIVX, VMSLTU, File::Vec, X, V),
    vector(OPIVI, VMSLE, File::Vec, Imm, V),
    vector(OPIVI, VSLIDEDOWN, File::Vec, Imm, V),
    vector(OPIVI, VSLIDEUP, File::Vec, Imm, V).reserving(Overlap::NotVs2),
    vector(OPIVV, VRGATHER, File::Vec, V, V).reserving(Overlap::NotSources),
    vector(OPIVI, VRGATHER, File::Vec, Imm, V).reserving(Overlap::NotVs2),
    vector(OPMVV, VMUL, File::Vec, V, V),
    vector(OPMVX, VMULH, File::Vec, X, V),
    vector(OPMVV, VDIV, File::Vec, V, V),
    vector(OPMVX, VREM, File::Vec, X, V),
    vector(OPMVV, VMACC, File::Vec, V, V),
    // vslide1down.vx: OPMVX shares vslidedown's funct6.
    vector(OPMVX, VSLIDEDOWN, File::Vec, X, V),
    vector(OPMVV, VREDSUM, File::Vec, V, V),
    vector(OPMVV, VREDMAX, File::Vec, V, V),
    vector(OPMVV, VMAND, File::Vec, V, V),
    vector(OPMVV, VWXUNARY0, File::Int, Code(VWXUNARY0_VMV_X_S), V),
    vector(OPMVV, VWXUNARY0, File::Int, Code(VWXUNARY0_VCPOP_M), V),
    vector(OPMVV, VWXUNARY0, File::Int, Code(VWXUNARY0_VFIRST_M), V),
    vector(OPMVV, VXUNARY0, File::Vec, Code(VXUNARY0_VZEXT_VF2), V).reserving(Overlap::NotVs2),
    vector(OPMVV, VXUNARY0, File::Vec, Code(VXUNARY0_VSEXT_VF4), V).reserving(Overlap::NotVs2),
    vector(OPMVV, VMUNARY0, File::Vec, Code(VMUNARY0_VID_V), Code(VID_V_VS2)),
    vector(OPMVX, VRXUNARY0, File::Vec, X, Code(VMV_S_X)),
    vector(OPFVV, VFADD, File::Vec, V, V),
    vector(OPFVF, VFMUL, File::Vec, F, V),
    vector(OPFVV, VFDIV, File::Vec, V, V),
    vector(OPFVF, VFMACC, File::Vec, F, V),
    vector(OPFVV, VFREDUSUM, File::Vec, V, V),
    vector(OPFVV, VFREDOSUM, File::Vec, V, V),
    vector(OPFVF, VMFLT, File::Vec, F, V),
    vector(OPFVV, VFUNARY1, File::Vec, Code(VFSQRT_V), V),
    vector(OPFVV, VFUNARY0, File::Vec, Code(VFCVT_X_F_V), V),
    vector(OPFVV, VWFUNARY0, File::Fp, Code(VWFUNARY0_VFMV_F_S), V),
    vector(OPFVF, VRFUNARY0, File::Vec, F, Code(VRFUNARY0_VFMV_S_F)),
    crypto(VCRYPTO_VV, Code(VAES_VS1_EF), V),
    crypto(VCRYPTO_VV, Code(VAES_VS1_DM), V),
    crypto(VAES_KF1, Imm, V),
    crypto(VAES_KF2, Imm, V),
    crypto(VGHSH, V, V),
    crypto(VCRYPTO_VV, Code(VGMUL_VS1), V),
    crypto(VSM4_K, Imm, V),
    crypto(VCRYPTO_VV, Code(VSM4_VS1_R), V),
    crypto(VSHA2_MS, V, V).reserving(Overlap::NotSources),
    crypto(VSHA2_CH, V, V).reserving(Overlap::NotSources),
    crypto(VSHA2_CL, V, V).reserving(Overlap::NotSources),
];

/// The register files an operation's `rd`, `rs1`, `rs2` and `rs3` fields
/// name registers in; `None` for a field that holds something else. An
/// OP-V instruction's vd, vs1 and vs2 fields sit where `rd`, `rs1` and
/// `rs2` do.
struct RegisterFields {
    rd: File,
    rs1: Option<File>,
    rs2: Option<File>,
    rs3: Option<File>,
}

impl Field {
    const fn file(self) -> Option<File> {
        match self {
            Reg(file) => Some(file),
            Imm | Code(_) => None,
        }
    }
}

impl Op {
    const fn register_fields(self) -> RegisterFields {
        let (rd, rs1, rs2, rs3) = match self {
            IntReg { .. } => (File::Int, Some(File::Int), Some(File::Int), None),
            IntImm { .. } | Shift { .. } | IntUnary { .. } => {
                (File::Int, Some(File::Int), None, None)
            }
            Upper { .. } => (File::Int, None, None, None),
            FpBinary { .. } => (File::Fp, Some(File::Fp), Some(File::Fp), None),
            FpUnary { src, dst, .. } => (dst, Some(src), None, None),
            Fma => (File::Fp, Some(File::Fp), Some(File::Fp), Some(File::Fp)),
            Vector { vd, vs1, vs2, .. } => (vd, vs1.file(), vs2.file(), None),
        };
        RegisterFields { rd, rs1, rs2, rs3 }
    }

    /// The immediate this operation encodes for the random `raw`.
    const fn immediate(self, raw: i32) -> i32 {
        match self {
            IntImm { .. } => raw << 20 >> 20,
            Shift { high, shamt_mask, .. } => high | (raw & shamt_mask),
            Upper { .. } => raw & 0xF_FFFF,
            Vector { .. } => raw & IMM5_MASK,
            IntReg { .. } | IntUnary { .. } | FpBinary { .. } | FpUnary { .. } | Fma => 0,
        }
    }
}

/// The `vtype` a program's vector instructions run under.
#[derive(Clone, Copy, Debug)]
struct VectorShape {
    /// The `vsew` field: 32-bit or 64-bit elements, wide enough for every
    /// FP operation and extension here.
    vsew: u32,
    /// The `vlmul` field, LMUL's log2.
    lmul_log2: u32,
}

impl VectorShape {
    /// The vector registers in each group.
    const fn lmul(self) -> u32 {
        1 << self.lmul_log2
    }

    /// The register groups in the vector register file.
    const fn groups(self) -> u8 {
        VECTOR_REGISTERS >> self.lmul_log2
    }

    /// `vsetivli zero, AVL, e<SEW>, m<LMUL>, ta, ma`.
    const fn vsetivli(self) -> u32 {
        let tail_and_mask_agnostic = 0b11 << 6;
        let zimm = tail_and_mask_agnostic | self.vsew << 3 | self.lmul_log2;
        0b11 << 30 | zimm << 20 | AVL << 15 | OPCFG << 12 | OP_V
    }
}

/// An instruction of a generated program: register numbers for each field,
/// whether or not its operation uses the field for a register. A vector
/// register field holds a register group's index.
#[derive(Clone, Copy, Debug)]
struct Inst {
    op: Op,
    rd: u8,
    rs1: u8,
    rs2: u8,
    rs3: u8,
    raw_immediate: i32,
}

impl Inst {
    fn encode(self, shape: VectorShape) -> u32 {
        let reg = RegIdx::new;
        let (rd, rs1, rs2) = (reg(self.rd), reg(self.rs1), reg(self.rs2));
        let imm = self.op.immediate(self.raw_immediate);
        match self.op {
            IntReg { opcode, funct3, funct7 } => {
                RType { opcode, funct3, funct7, rd, rs1, rs2 }.encode()
            }
            IntImm { opcode, funct3 } | Shift { opcode, funct3, .. } => {
                IType { opcode, funct3, rd, rs1, imm }.encode()
            }
            IntUnary { imm12 } => {
                IType { opcode: OP_IMM, funct3: SLL, rd, rs1, imm: imm12.cast_signed() }.encode()
            }
            Upper { opcode } => UType { opcode, rd, imm20: imm.cast_unsigned() }.encode(),
            FpBinary { funct7 } => {
                RType { opcode: OP_FP, funct3: FP_FUNCT3, funct7, rd, rs1, rs2 }.encode()
            }
            FpUnary { funct7, code, .. } => {
                RType { opcode: OP_FP, funct3: FP_FUNCT3, funct7, rd, rs1, rs2: reg(code) }.encode()
            }
            Fma => {
                let rs3 = reg(self.rs3);
                R4Type { opcode: OP_FMADD, funct3: FP_FUNCT3, fmt: FMT_DOUBLE, rd, rs1, rs2, rs3 }
                    .encode()
            }
            Vector { opcode, funct3, funct6, vd, vs1, vs2, .. } => {
                let field = |field: Field, value: u8| match field {
                    Reg(File::Vec) => u32::from(value) * shape.lmul(),
                    Reg(File::Int | File::Fp) => u32::from(value),
                    Imm => imm.cast_unsigned(),
                    Code(code) => u32::from(code),
                };
                let (vd, vs1, vs2) =
                    (field(Reg(vd), self.rd), field(vs1, self.rs1), field(vs2, self.rs2));
                funct6 << 26 | UNMASKED | vs2 << 20 | vs1 << 15 | funct3 << 12 | vd << 7 | opcode
            }
        }
    }

    /// Whether this instruction's destination group leaves alone the
    /// sources its encoding reserves an overlap with.
    const fn has_legal_overlap(self) -> bool {
        match self.op {
            Vector { overlap: Overlap::NotVs2, .. } => self.rd != self.rs2,
            Vector { overlap: Overlap::NotSources, .. } => {
                self.rd != self.rs1 && self.rd != self.rs2
            }
            _ => true,
        }
    }

    /// This instruction with each register field renamed through the
    /// table of its file.
    fn renamed(self, renaming: &Renaming) -> Self {
        let rename = |file: File, reg: u8| {
            let table = match file {
                File::Int => &renaming.int,
                File::Fp => &renaming.fp,
                File::Vec => &renaming.vec_groups,
            };
            table[usize::from(reg)]
        };
        let fields = self.op.register_fields();
        let field = |file: Option<File>, reg: u8| file.map_or(reg, |file| rename(file, reg));
        Self {
            rd: rename(fields.rd, self.rd),
            rs1: field(fields.rs1, self.rs1),
            rs2: field(fields.rs2, self.rs2),
            rs3: field(fields.rs3, self.rs3),
            ..self
        }
    }
}

/// A generated program: its instructions and the `vtype` they run under.
#[derive(Clone, Debug)]
struct Program {
    shape: VectorShape,
    insts: Vec<Inst>,
}

/// A consistent renaming of a program's registers: a table per file, the
/// vector one over register groups.
struct Renaming {
    int: Vec<u8>,
    fp: Vec<u8>,
    vec_groups: Vec<u8>,
}

impl Renaming {
    /// The renaming `int` and `fp` give and, for `shape`'s register groups,
    /// the order the group indices fall in `vec`, a permutation of every
    /// vector register.
    fn new(int: Vec<u8>, fp: Vec<u8>, vec: &[u8], shape: VectorShape) -> Self {
        let vec_groups = vec.iter().copied().filter(|&reg| reg < shape.groups()).collect();
        Self { int, fp, vec_groups }
    }
}

fn shape() -> impl Strategy<Value = VectorShape> {
    const LARGEST_LMUL_LOG2: u32 = 2;
    (select(vec![E32, E64]), 0..=LARGEST_LMUL_LOG2)
        .prop_map(|(vsew, lmul_log2)| VectorShape { vsew, lmul_log2 })
}

/// An instruction for `shape`, its vector register fields holding group
/// indices.
fn inst(shape: VectorShape) -> impl Strategy<Value = Inst> {
    let reg = || 0u8..32;
    let ops: Vec<Op> = OPS.iter().copied().filter(|op| op.is_defined_for(shape)).collect();
    (select(ops), reg(), reg(), reg(), reg(), any::<i32>())
        .prop_map(move |(op, rd, rs1, rs2, rs3, raw_immediate)| {
            let fields = op.register_fields();
            let group = |file: Option<File>, reg: u8| {
                if file == Some(File::Vec) { reg % shape.groups() } else { reg }
            };
            Inst {
                op,
                rd: group(Some(fields.rd), rd),
                rs1: group(fields.rs1, rs1),
                rs2: group(fields.rs2, rs2),
                rs3,
                raw_immediate,
            }
        })
        .prop_filter("a reserved register overlap", |inst| inst.has_legal_overlap())
}

fn program() -> impl Strategy<Value = Program> {
    shape().prop_flat_map(|shape| {
        vec(inst(shape), 1..=MAX_INSTRUCTIONS).prop_map(move |insts| Program { shape, insts })
    })
}

/// A permutation of the integer registers that keeps x0, which reads as
/// zero, in place.
fn int_renaming() -> impl Strategy<Value = Vec<u8>> {
    Just((1..32).collect::<Vec<u8>>())
        .prop_shuffle()
        .prop_map(|others| std::iter::once(0).chain(others).collect())
}

/// A permutation of the FP or vector registers, none of which is special
/// here.
fn permutation() -> impl Strategy<Value = Vec<u8>> {
    Just((0..32).collect::<Vec<u8>>()).prop_shuffle()
}

/// Cycles until `program` retires on the minimal machine with `backend`.
fn cycles_to_retire(backend: BackendKind, program: &Program) -> u64 {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    let mut words = vec![program.shape.vsetivli()];
    words.extend(program.insts.iter().map(|inst| inst.encode(program.shape)));
    words.push(InstructionBuilder::new().jal(X0, 0).build());
    let retired = words.len() as u64 - 1;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &words);

    ctx.run_until(CYCLE_LIMIT, |ctx| ctx.state().harts[0].instructions_retired >= retired)
        .expect("the program retired")
}

proptest! {
    #![proptest_config(seeded_cases(CASES))]

    #[test]
    fn a_programs_cycle_count_does_not_depend_on_its_immediates(
        program in program(),
        immediates in vec(any::<i32>(), MAX_INSTRUCTIONS),
    ) {
        let insts = program
            .insts
            .iter()
            .zip(&immediates)
            .map(|(&inst, &raw_immediate)| Inst { raw_immediate, ..inst })
            .collect();
        let changed = Program { shape: program.shape, insts };

        for backend in BACKENDS {
            prop_assert_eq!(
                cycles_to_retire(backend, &program),
                cycles_to_retire(backend, &changed),
                "{:?}",
                backend
            );
        }
    }

    #[test]
    fn a_programs_cycle_count_does_not_depend_on_which_registers_it_names(
        program in program(),
        int in int_renaming(),
        fp in permutation(),
        vec in permutation(),
    ) {
        let renaming = Renaming::new(int, fp, &vec, program.shape);
        let insts = program.insts.iter().map(|inst| inst.renamed(&renaming)).collect();
        let renamed = Program { shape: program.shape, insts };

        for backend in BACKENDS {
            prop_assert_eq!(
                cycles_to_retire(backend, &program),
                cycles_to_retire(backend, &renamed),
                "{:?}",
                backend
            );
        }
    }
}
