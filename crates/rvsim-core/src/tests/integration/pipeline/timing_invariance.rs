//! Timing properties of branch-free, memory-free scalar programs.
//!
//! No scalar unit's latency depends on the values it computes, so such a
//! program takes as many cycles whatever its immediates hold, and whatever
//! registers it names as long as it renames them consistently. An
//! instruction that waited on a register it does not read breaks both.

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
use crate::isa::reg::RegIdx;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::X0;
use crate::tests::support::seeded_cases;

const CASES: u32 = 512;
const MAX_INSTRUCTIONS: usize = 24;
const CYCLE_LIMIT: u64 = 20_000;
const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

/// The `fcvt` rs2 code for a 64-bit signed integer.
const FCVT_LONG: u8 = 2;
/// The funct3 of every FP operation here: round to nearest, ties to even,
/// for those that round, and the value `fmv` requires.
const FP_FUNCT3: u32 = 0;
/// The `fmt` field of a double-precision fused multiply-add.
const FMT_DOUBLE: u32 = 1;
/// `srai`'s funct6, which sits in its immediate above the shift amount.
const SRAI_HIGH_BITS: i32 = 0x400;

/// A register file an encoding's field names a register in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum File {
    Int,
    Fp,
}

/// The operations the programs draw from: every scalar unit, and encodings
/// whose rs1 or rs2 field holds an immediate or a sub-opcode.
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
}

use Op::{Fma, FpBinary, FpUnary, IntImm, IntReg, IntUnary, Shift, Upper};

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
];

/// The register files an operation's `rd`, `rs1`, `rs2` and `rs3` fields
/// name registers in; `None` for a field that holds something else.
struct RegisterFields {
    rd: File,
    rs1: Option<File>,
    rs2: Option<File>,
    rs3: Option<File>,
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
        };
        RegisterFields { rd, rs1, rs2, rs3 }
    }

    /// The immediate this operation encodes for the random `raw`.
    const fn immediate(self, raw: i32) -> i32 {
        match self {
            IntImm { .. } => raw << 20 >> 20,
            Shift { high, shamt_mask, .. } => high | (raw & shamt_mask),
            Upper { .. } => raw & 0xF_FFFF,
            IntReg { .. } | IntUnary { .. } | FpBinary { .. } | FpUnary { .. } | Fma => 0,
        }
    }
}

/// An instruction of a generated program: register numbers for each field,
/// whether or not its operation uses the field for a register.
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
    fn encode(self) -> u32 {
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
        }
    }

    /// This instruction with each register field renamed through the
    /// table of its file.
    fn renamed(self, int: &[u8], fp: &[u8]) -> Self {
        let rename = |file: File, reg: u8| match file {
            File::Int => int[usize::from(reg)],
            File::Fp => fp[usize::from(reg)],
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

fn inst() -> impl Strategy<Value = Inst> {
    let reg = || 0u8..32;
    (select(OPS), reg(), reg(), reg(), reg(), any::<i32>()).prop_map(
        |(op, rd, rs1, rs2, rs3, raw_immediate)| Inst { op, rd, rs1, rs2, rs3, raw_immediate },
    )
}

fn program() -> impl Strategy<Value = Vec<Inst>> {
    vec(inst(), 1..=MAX_INSTRUCTIONS)
}

/// A permutation of the integer registers that keeps x0, which reads as
/// zero, in place.
fn int_renaming() -> impl Strategy<Value = Vec<u8>> {
    Just((1..32).collect::<Vec<u8>>())
        .prop_shuffle()
        .prop_map(|others| std::iter::once(0).chain(others).collect())
}

/// A permutation of the FP registers, none of which is special.
fn fp_renaming() -> impl Strategy<Value = Vec<u8>> {
    Just((0..32).collect::<Vec<u8>>()).prop_shuffle()
}

/// Cycles until `program` retires on the minimal machine with `backend`.
fn cycles_to_retire(backend: BackendKind, program: &[Inst]) -> u64 {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    let mut words: Vec<u32> = program.iter().map(|inst| inst.encode()).collect();
    words.push(InstructionBuilder::new().jal(X0, 0).build());
    let retired = program.len() as u64;
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
        let changed: Vec<Inst> = program
            .iter()
            .zip(&immediates)
            .map(|(&inst, &raw_immediate)| Inst { raw_immediate, ..inst })
            .collect();

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
        fp in fp_renaming(),
    ) {
        let renamed: Vec<Inst> = program.iter().map(|inst| inst.renamed(&int, &fp)).collect();

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
