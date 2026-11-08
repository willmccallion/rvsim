//! Scalar instruction decoding.

use super::vector;
use super::{VEC_WIDTH_8, VEC_WIDTH_16, VEC_WIDTH_32, VEC_WIDTH_64};
use crate::common::VirtAddr;
use crate::exec::signals::{BranchCond, ControlFlow, ControlSignals, JumpKind, OpASrc, OpBSrc};
use crate::isa::encoding::privileged as sys_ops;
use crate::isa::encoding::rv64a::{
    AQ as AMO_AQ, RL as AMO_RL, funct3 as a_funct3, funct5 as a_funct5, opcodes as a_opcodes,
};
use crate::isa::encoding::rv64bk::{funct3 as b_funct3, funct7 as b_funct7};
use crate::isa::encoding::rv64d::{funct7 as d_funct7, opcodes as d_opcodes};
use crate::isa::encoding::rv64f::{funct3 as f_funct3, funct7 as f_funct7, opcodes as f_opcodes};
use crate::isa::encoding::rv64i::{funct3 as i_funct3, funct7 as i_funct7, opcodes as i_opcodes};
use crate::isa::encoding::rv64m::{funct3 as m_funct3, opcodes as m_opcodes};
use crate::isa::encoding::rv64zfh::funct7 as h_funct7;
use crate::isa::encoding::zicboz;
use crate::isa::fence::Fence;
use crate::isa::fp::{FpFormat, Rounding};
use crate::isa::instruction::{Decoded, InstructionBits};
use crate::isa::op::AmoOp;
use crate::isa::op::pseudo::PseudoOp;
use crate::isa::op::{
    AluOp, ArithOp, AtomicOp, BitOp, ClmulOp, CountOp, CsrOp, DivOp, FmaSigns, FpArithOp,
    FpCompareOp, FpOp, IntAluOp, IntWidth, LogicOp, MemWidth, MinMaxOp, MulOp, OperandFormat,
    PackOp, RotateOp, ShiftAddOp, ShiftOp, SignInjectOp, Signedness, SingleBitOp, SystemOp,
};
use crate::isa::privileged::Trap;

/// funct6 (bits 31:26) of the 64-bit shift-immediate encodings, whose low
/// six immediate bits are the shift amount.
const SLLI_SRLI_FUNCT6: u32 = i_funct7::DEFAULT >> 1;
const SRAI_FUNCT6: u32 = i_funct7::SRA >> 1;
const RORI_FUNCT6: u32 = b_funct7::ROTATE_RIGHT >> 1;
const BCLRI_FUNCT6: u32 = b_funct7::BCLR >> 1;
const BEXTI_FUNCT6: u32 = b_funct7::BEXT >> 1;
const BINVI_FUNCT6: u32 = b_funct7::BINV >> 1;
const BSETI_FUNCT6: u32 = b_funct7::BSET >> 1;
const SLLI_UW_FUNCT6: u32 = b_funct7::SLLI_UW >> 1;

/// Floating-point width encoding for 16-bit half operations (Zfh).
const FP_WIDTH_HALF: u32 = 0x1;

/// Floating-point width encoding for 32-bit word operations.
const FP_WIDTH_WORD: u32 = 0x2;

/// Floating-point width encoding for 64-bit double operations.
const FP_WIDTH_DOUBLE: u32 = 0x3;

/// Floating-point format encoding for single-precision (32-bit).
const FP_FMT_SINGLE: u32 = 0;

/// Floating-point format encoding for double-precision (64-bit).
const FP_FMT_DOUBLE: u32 = 1;

/// Floating-point format encoding for half-precision (16-bit, Zfh).
const FP_FMT_HALF: u32 = 2;

/// The bits of an SFENCE.VMA that are not rs1 or rs2: funct7, funct3, rd
/// and the opcode, which identify it.
const SFENCE_VMA_MASK: u32 = 0xFE00_7FFF;

/// Decodes a scalar instruction (RV64IMAFDC, Zb*, Zk*, Zicsr, Zicbo*,
/// Zfh) into `c`, which starts with `alu: Add`, `a_src: Reg1` and
/// `b_src: Imm`.
pub(super) fn decode(
    c: &mut ControlSignals,
    inst: u32,
    pc: VirtAddr,
    d: &Decoded,
) -> Result<(), Trap> {
    let illegal = || Trap::IllegalInstruction(inst);
    match d.opcode {
        i_opcodes::OP_LUI => {
            c.reg_write = true;
            c.a_src = OpASrc::Zero;
        }
        i_opcodes::OP_AUIPC => {
            c.reg_write = true;
            c.a_src = OpASrc::Pc;
        }
        i_opcodes::OP_JAL => {
            c.reg_write = true;
            c.control_flow = ControlFlow::Jump(JumpKind::Jal);
        }
        i_opcodes::OP_JALR if d.funct3 == i_funct3::JALR => {
            c.reg_write = true;
            c.control_flow = ControlFlow::Jump(JumpKind::Jalr);
        }
        i_opcodes::OP_BRANCH => {
            c.control_flow = ControlFlow::Branch(branch_cond(d.funct3).ok_or_else(illegal)?);
            c.b_src = OpBSrc::Reg2;
        }
        i_opcodes::OP_LOAD => {
            (c.width, c.signed_load) = load_width(d.funct3).ok_or_else(illegal)?;
            c.reg_write = true;
            c.mem_read = true;
        }
        i_opcodes::OP_STORE => {
            c.width = store_width(d.funct3).ok_or_else(illegal)?;
            c.mem_write = true;
        }
        i_opcodes::OP_IMM => {
            c.reg_write = true;
            c.alu = AluOp::Int(op_imm(inst, d).ok_or_else(illegal)?);
        }
        i_opcodes::OP_IMM_32 => {
            c.reg_write = true;
            c.alu = AluOp::Int(op_imm_32(inst, d).ok_or_else(illegal)?);
            c.format = OperandFormat::Int(word_op_width(c.alu));
        }
        i_opcodes::OP_REG => {
            c.reg_write = true;
            c.b_src = OpBSrc::Reg2;
            c.alu = AluOp::Int(op(d).ok_or_else(illegal)?);
        }
        i_opcodes::OP_REG_32 => {
            c.reg_write = true;
            c.b_src = OpBSrc::Reg2;
            c.alu = AluOp::Int(op_32(d).ok_or_else(illegal)?);
            c.format = OperandFormat::Int(word_op_width(c.alu));
        }
        a_opcodes::OP_AMO => decode_amo(c, inst, d).ok_or_else(illegal)?,
        f_opcodes::OP_LOAD_FP if is_vector_width(d.funct3) => {
            vector::decode_vec_load(inst, d.funct3, c)?;
        }
        f_opcodes::OP_LOAD_FP => {
            let (width, format) = fp_mem_width(d.funct3).ok_or_else(illegal)?;
            c.fp_reg_write = true;
            c.mem_read = true;
            c.width = width;
            c.format = OperandFormat::Fp(format);
        }
        f_opcodes::OP_STORE_FP if is_vector_width(d.funct3) => {
            vector::decode_vec_store(inst, d.funct3, c)?;
        }
        f_opcodes::OP_STORE_FP => {
            let (width, format) = fp_mem_width(d.funct3).ok_or_else(illegal)?;
            c.mem_write = true;
            c.rs2_fp = true;
            c.width = width;
            c.format = OperandFormat::Fp(format);
        }
        f_opcodes::OP_FP => {
            c.format = OperandFormat::Fp(fp_format(d.funct7).ok_or_else(illegal)?);
            c.rs1_fp = true;
            c.rs2_fp = true;
            c.fp_reg_write = true;
            c.b_src = OpBSrc::Reg2;
            let op = op_fp(c, d).ok_or_else(illegal)?;
            c.alu = AluOp::Fp(op);
            if rounds(op) {
                c.fp_rm = Some(rounding(inst, d)?);
            }
        }
        d_opcodes::OP_FMADD => decode_fused(c, inst, d, FmaSigns::Macc)?,
        d_opcodes::OP_FMSUB => decode_fused(c, inst, d, FmaSigns::MSac)?,
        d_opcodes::OP_FNMADD => decode_fused(c, inst, d, FmaSigns::NMacc)?,
        d_opcodes::OP_FNMSUB => decode_fused(c, inst, d, FmaSigns::NMSac)?,
        sys_ops::OP_SYSTEM => decode_system(c, inst, pc, d)?,
        sys_ops::OP_PSEUDO => c.system_op = SystemOp::Pseudo(PseudoOp::of(inst)),
        i_opcodes::OP_MISC_MEM => c.system_op = misc_mem_op(d).ok_or_else(illegal)?,
        _ => return Err(illegal()),
    }
    Ok(())
}

/// The comparison a BRANCH `funct3` names.
const fn branch_cond(funct3: u32) -> Option<BranchCond> {
    Some(match funct3 {
        i_funct3::BEQ => BranchCond::Eq,
        i_funct3::BNE => BranchCond::Ne,
        i_funct3::BLT => BranchCond::Lt,
        i_funct3::BGE => BranchCond::Ge,
        i_funct3::BLTU => BranchCond::Ltu,
        i_funct3::BGEU => BranchCond::Geu,
        _ => return None,
    })
}

/// A LOAD's width and whether it sign-extends.
const fn load_width(funct3: u32) -> Option<(MemWidth, bool)> {
    Some(match funct3 {
        i_funct3::LB => (MemWidth::Byte, true),
        i_funct3::LH => (MemWidth::Half, true),
        i_funct3::LW => (MemWidth::Word, true),
        i_funct3::LD => (MemWidth::Double, true),
        i_funct3::LBU => (MemWidth::Byte, false),
        i_funct3::LHU => (MemWidth::Half, false),
        i_funct3::LWU => (MemWidth::Word, false),
        _ => return None,
    })
}

/// A STORE's width.
const fn store_width(funct3: u32) -> Option<MemWidth> {
    Some(match funct3 {
        i_funct3::SB => MemWidth::Byte,
        i_funct3::SH => MemWidth::Half,
        i_funct3::SW => MemWidth::Word,
        i_funct3::SD => MemWidth::Double,
        _ => return None,
    })
}

/// Whether a LOAD-FP or STORE-FP `funct3` is a vector access's width.
const fn is_vector_width(funct3: u32) -> bool {
    matches!(funct3, VEC_WIDTH_8 | VEC_WIDTH_16 | VEC_WIDTH_32 | VEC_WIDTH_64)
}

/// A scalar FP load's or store's width and format.
const fn fp_mem_width(funct3: u32) -> Option<(MemWidth, FpFormat)> {
    Some(match funct3 {
        FP_WIDTH_HALF => (MemWidth::Half, FpFormat::Half),
        FP_WIDTH_WORD => (MemWidth::Word, FpFormat::Single),
        FP_WIDTH_DOUBLE => (MemWidth::Double, FpFormat::Double),
        _ => return None,
    })
}

/// Decodes an LR, SC or AMO into `c`; `None` for a reserved encoding.
fn decode_amo(c: &mut ControlSignals, inst: u32, d: &Decoded) -> Option<()> {
    c.width = match d.funct3 {
        a_funct3::WIDTH_32 => MemWidth::Word,
        a_funct3::WIDTH_64 => MemWidth::Double,
        _ => return None,
    };
    let op = atomic_op(d.funct7 >> 2)?;
    if op == AtomicOp::Lr && !d.rs2.is_zero() {
        return None;
    }
    c.atomic_op = Some(op);
    c.b_src = OpBSrc::Zero;
    c.acquire = inst & AMO_AQ != 0;
    c.release = inst & AMO_RL != 0;
    c.mem_read = true;
    c.mem_write = op != AtomicOp::Lr;
    c.reg_write = true;
    // AMO and LR always sign-extend the loaded old value. SC writes
    // a 0/1 success code and overrides load_data in memory2.
    c.signed_load = true;
    Some(())
}

/// The atomic operation an AMO's `funct5` names.
const fn atomic_op(funct5: u32) -> Option<AtomicOp> {
    Some(match funct5 {
        a_funct5::LR => AtomicOp::Lr,
        a_funct5::SC => AtomicOp::Sc,
        a_funct5::AMOSWAP => AtomicOp::Amo(AmoOp::Swap),
        a_funct5::AMOADD => AtomicOp::Amo(AmoOp::Add),
        a_funct5::AMOXOR => AtomicOp::Amo(AmoOp::Xor),
        a_funct5::AMOAND => AtomicOp::Amo(AmoOp::And),
        a_funct5::AMOOR => AtomicOp::Amo(AmoOp::Or),
        a_funct5::AMOMIN => AtomicOp::Amo(AmoOp::Min),
        a_funct5::AMOMAX => AtomicOp::Amo(AmoOp::Max),
        a_funct5::AMOMINU => AtomicOp::Amo(AmoOp::MinU),
        a_funct5::AMOMAXU => AtomicOp::Amo(AmoOp::MaxU),
        _ => return None,
    })
}

/// The integer an `fcvt`'s `rs2` field names.
#[derive(Clone, Copy)]
enum FcvtInt {
    W,
    Wu,
    L,
    Lu,
}

impl FcvtInt {
    const fn of(rs2: u8) -> Option<Self> {
        Some(match rs2 {
            0 => Self::W,
            1 => Self::Wu,
            2 => Self::L,
            3 => Self::Lu,
            _ => return None,
        })
    }
}

/// The floating-point operation an OP-FP word `d` encodes, with where its
/// operands come from and its result goes set in `c`; `None` for a
/// reserved encoding.
fn op_fp(c: &mut ControlSignals, d: &Decoded) -> Option<FpOp> {
    let rs2 = d.rs2.as_u8();
    Some(match d.funct7 {
        f_funct7::FADD | d_funct7::FADD_D | h_funct7::FADD_H => FpOp::Arith(FpArithOp::Add),
        f_funct7::FSUB | d_funct7::FSUB_D | h_funct7::FSUB_H => FpOp::Arith(FpArithOp::Sub),
        f_funct7::FMUL | d_funct7::FMUL_D | h_funct7::FMUL_H => FpOp::Arith(FpArithOp::Mul),
        f_funct7::FDIV | d_funct7::FDIV_D | h_funct7::FDIV_H => FpOp::Arith(FpArithOp::Div),
        f_funct7::FSQRT | d_funct7::FSQRT_D | h_funct7::FSQRT_H if rs2 == 0 => FpOp::Sqrt,
        f_funct7::FSGNJ | d_funct7::FSGNJ_D | h_funct7::FSGNJ_H => match d.funct3 {
            f_funct3::FSGNJ => FpOp::SignInject(SignInjectOp::Copy),
            f_funct3::FSGNJN => FpOp::SignInject(SignInjectOp::Negate),
            f_funct3::FSGNJX => FpOp::SignInject(SignInjectOp::Xor),
            _ => return None,
        },
        f_funct7::FMIN_MAX | d_funct7::FMIN_MAX_D | h_funct7::FMIN_MAX_H => match d.funct3 {
            f_funct3::FMIN => FpOp::Min,
            f_funct3::FMAX => FpOp::Max,
            _ => return None,
        },
        f_funct7::FCMP | d_funct7::FCMP_D | h_funct7::FCMP_H => {
            writes_int_register(c);
            match d.funct3 {
                f_funct3::FEQ => FpOp::Compare(FpCompareOp::Eq),
                f_funct3::FLT => FpOp::Compare(FpCompareOp::Lt),
                f_funct3::FLE => FpOp::Compare(FpCompareOp::Le),
                _ => return None,
            }
        }
        f_funct7::FCLASS_MV_X_F | d_funct7::FCLASS_MV_X_D | h_funct7::FCLASS_MV_X_H if rs2 == 0 => {
            writes_int_register(c);
            match d.funct3 {
                f_funct3::FMV_X_W => FpOp::MoveToInt,
                f_funct3::FCLASS => FpOp::Class,
                _ => return None,
            }
        }
        f_funct7::FMV_F_X | d_funct7::FMV_D_X | h_funct7::FMV_H_X
            if rs2 == 0 && d.funct3 == f_funct3::FMV_X_W =>
        {
            c.rs1_fp = false;
            FpOp::MoveFromInt
        }
        f_funct7::FCVT_W_F | d_funct7::FCVT_W_D | h_funct7::FCVT_W_H => {
            writes_int_register(c);
            match FcvtInt::of(rs2)? {
                FcvtInt::W => FpOp::ToInt(IntWidth::Word, Signedness::Signed),
                FcvtInt::Wu => FpOp::ToInt(IntWidth::Word, Signedness::Unsigned),
                FcvtInt::L => FpOp::ToInt(IntWidth::Double, Signedness::Signed),
                FcvtInt::Lu => FpOp::ToInt(IntWidth::Double, Signedness::Unsigned),
            }
        }
        f_funct7::FCVT_F_W | d_funct7::FCVT_D_W | h_funct7::FCVT_H_W => {
            c.rs1_fp = false;
            match FcvtInt::of(rs2)? {
                FcvtInt::W => FpOp::FromInt(IntWidth::Word, Signedness::Signed),
                FcvtInt::Wu => FpOp::FromInt(IntWidth::Word, Signedness::Unsigned),
                FcvtInt::L => FpOp::FromInt(IntWidth::Double, Signedness::Signed),
                FcvtInt::Lu => FpOp::FromInt(IntWidth::Double, Signedness::Unsigned),
            }
        }
        // FP↔FP conversion: the target fmt is funct7's, the source fmt rs2's.
        f_funct7::FCVT_DS => {
            c.format = OperandFormat::Fp(FpFormat::Double);
            match u32::from(rs2) {
                FP_FMT_SINGLE => FpOp::FromFloat(FpFormat::Single),
                FP_FMT_HALF => FpOp::FromFloat(FpFormat::Half),
                _ => return None,
            }
        }
        d_funct7::FCVT_S_D => {
            c.format = OperandFormat::Fp(FpFormat::Single);
            match u32::from(rs2) {
                FP_FMT_DOUBLE => FpOp::FromFloat(FpFormat::Double),
                FP_FMT_HALF => FpOp::FromFloat(FpFormat::Half),
                _ => return None,
            }
        }
        h_funct7::FCVT_H_FP => {
            c.format = OperandFormat::Fp(FpFormat::Half);
            match u32::from(rs2) {
                FP_FMT_SINGLE => FpOp::FromFloat(FpFormat::Single),
                FP_FMT_DOUBLE => FpOp::FromFloat(FpFormat::Double),
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// Sends an FP instruction's result to the integer register file.
const fn writes_int_register(c: &mut ControlSignals) {
    c.fp_reg_write = false;
    c.reg_write = true;
}

/// Decodes a fused multiply-add with the `signs` into `c`.
fn decode_fused(
    c: &mut ControlSignals,
    inst: u32,
    d: &Decoded,
    signs: FmaSigns,
) -> Result<(), Trap> {
    c.rs1_fp = true;
    c.rs2_fp = true;
    c.rs3_fp = true;
    c.fp_reg_write = true;
    c.b_src = OpBSrc::Reg2;
    c.format = OperandFormat::Fp(fp_format(d.funct7).ok_or(Trap::IllegalInstruction(inst))?);
    c.fp_rm = Some(rounding(inst, d)?);
    c.alu = AluOp::Fp(FpOp::Fma(signs));
    Ok(())
}

/// Decodes a SYSTEM instruction into `c`: SFENCE.VMA, a privileged
/// instruction, or a CSR access.
fn decode_system(c: &mut ControlSignals, inst: u32, pc: VirtAddr, d: &Decoded) -> Result<(), Trap> {
    if inst & SFENCE_VMA_MASK == sys_ops::SFENCE_VMA {
        c.system_op = SystemOp::SfenceVma;
        return Ok(());
    }
    if d.funct3 == 0 {
        c.system_op = match d.raw {
            sys_ops::EBREAK => return Err(Trap::Breakpoint(pc.val())),
            sys_ops::MRET => SystemOp::Mret,
            sys_ops::SRET => SystemOp::Sret,
            sys_ops::WFI => SystemOp::Wfi,
            sys_ops::ECALL => SystemOp::Ecall,
            _ => return Err(Trap::IllegalInstruction(inst)),
        };
        return Ok(());
    }
    c.csr_op = match d.funct3 {
        sys_ops::CSRRW => CsrOp::Rw,
        sys_ops::CSRRS => CsrOp::Rs,
        sys_ops::CSRRC => CsrOp::Rc,
        sys_ops::CSRRWI => CsrOp::Rwi,
        sys_ops::CSRRSI => CsrOp::Rsi,
        sys_ops::CSRRCI => CsrOp::Rci,
        _ => return Err(Trap::IllegalInstruction(inst)),
    };
    c.system_op = SystemOp::Csr;
    c.csr_addr = inst.csr();
    c.b_src = OpBSrc::Zero;
    c.reg_write = !d.rd.is_zero();
    Ok(())
}

/// The MISC-MEM operation `d` encodes: a fence or a cache-block operation.
const fn misc_mem_op(d: &Decoded) -> Option<SystemOp> {
    Some(match d.funct3 {
        i_funct3::FENCE => SystemOp::Fence(Fence::decode(d.raw)),
        i_funct3::FENCE_I => SystemOp::FenceI,
        i_funct3::CBO if d.rd.is_zero() => match d.imm {
            zicboz::CBO_ZERO_IMM => SystemOp::CboZero,
            zicboz::CBO_INVAL_IMM => SystemOp::CboInval,
            zicboz::CBO_CLEAN_IMM => SystemOp::CboClean,
            zicboz::CBO_FLUSH_IMM => SystemOp::CboFlush,
            _ => return None,
        },
        _ => return None,
    })
}

/// The 12-bit I-type immediate, which selects the Zbb/Zbkb unary ops.
const fn imm12(inst: u32) -> u32 {
    (inst >> b_funct3::I_IMM_SHIFT) & 0xFFF
}

/// The width a W-opcode operation computes at: Zba's `.uw` ops sit in the
/// W opcodes but produce a full 64-bit result.
const fn word_op_width(op: AluOp) -> IntWidth {
    match op {
        AluOp::Int(IntAluOp::Bit(BitOp::ShiftAdd(
            ShiftAddOp::AddUw
            | ShiftAddOp::Sh1AddUw
            | ShiftAddOp::Sh2AddUw
            | ShiftAddOp::Sh3AddUw
            | ShiftAddOp::SlliUw,
        ))) => IntWidth::Double,
        _ => IntWidth::Word,
    }
}

/// The format an FP instruction's `fmt` field (`funct7[1:0]`) names, or
/// `None` for the quad-precision encoding, which is not implemented.
const fn fp_format(funct7: u32) -> Option<FpFormat> {
    match funct7 & 0x3 {
        FP_FMT_SINGLE => Some(FpFormat::Single),
        FP_FMT_DOUBLE => Some(FpFormat::Double),
        FP_FMT_HALF => Some(FpFormat::Half),
        _ => None,
    }
}

/// The integer operation an OP-IMM word encodes, or `None` for a reserved one.
const fn op_imm(inst: u32, d: &Decoded) -> Option<IntAluOp> {
    let funct6 = d.funct7 >> 1;
    Some(match d.funct3 {
        i_funct3::ADD_SUB => IntAluOp::Arith(ArithOp::Add),
        i_funct3::SLT => IntAluOp::Logic(LogicOp::Slt),
        i_funct3::SLTU => IntAluOp::Logic(LogicOp::Sltu),
        i_funct3::XOR => IntAluOp::Logic(LogicOp::Xor),
        i_funct3::OR => IntAluOp::Logic(LogicOp::Or),
        i_funct3::AND => IntAluOp::Logic(LogicOp::And),
        i_funct3::SLL => match imm12(inst) {
            b_funct3::CLZ_IMM => IntAluOp::Bit(BitOp::Count(CountOp::Clz)),
            b_funct3::CTZ_IMM => IntAluOp::Bit(BitOp::Count(CountOp::Ctz)),
            b_funct3::CPOP_IMM => IntAluOp::Bit(BitOp::Count(CountOp::Cpop)),
            b_funct3::SEXT_B_IMM => IntAluOp::Bit(BitOp::SextB),
            b_funct3::SEXT_H_IMM => IntAluOp::Bit(BitOp::SextH),
            _ => match funct6 {
                SLLI_SRLI_FUNCT6 => IntAluOp::Shift(ShiftOp::Sll),
                BCLRI_FUNCT6 => IntAluOp::Bit(BitOp::SingleBit(SingleBitOp::Bclr)),
                BINVI_FUNCT6 => IntAluOp::Bit(BitOp::SingleBit(SingleBitOp::Binv)),
                BSETI_FUNCT6 => IntAluOp::Bit(BitOp::SingleBit(SingleBitOp::Bset)),
                _ => return None,
            },
        },
        i_funct3::SRL_SRA => match imm12(inst) {
            b_funct3::ORC_B_IMM => IntAluOp::Bit(BitOp::OrcB),
            b_funct3::REV8_IMM => IntAluOp::Bit(BitOp::Rev8),
            b_funct3::BREV8_IMM => IntAluOp::Bit(BitOp::Brev8),
            _ => match funct6 {
                SLLI_SRLI_FUNCT6 => IntAluOp::Shift(ShiftOp::Srl),
                SRAI_FUNCT6 => IntAluOp::Shift(ShiftOp::Sra),
                RORI_FUNCT6 => IntAluOp::Bit(BitOp::Rotate(RotateOp::Ror)),
                BEXTI_FUNCT6 => IntAluOp::Bit(BitOp::SingleBit(SingleBitOp::Bext)),
                _ => return None,
            },
        },
        _ => return None,
    })
}

/// The integer operation an OP-IMM-32 word encodes, or `None` for a reserved one.
const fn op_imm_32(inst: u32, d: &Decoded) -> Option<IntAluOp> {
    Some(match d.funct3 {
        i_funct3::ADD_SUB => IntAluOp::Arith(ArithOp::Add),
        i_funct3::SLL => match imm12(inst) {
            b_funct3::CLZ_IMM => IntAluOp::Bit(BitOp::Count(CountOp::Clz)),
            b_funct3::CTZ_IMM => IntAluOp::Bit(BitOp::Count(CountOp::Ctz)),
            b_funct3::CPOP_IMM => IntAluOp::Bit(BitOp::Count(CountOp::Cpop)),
            _ if d.funct7 == i_funct7::DEFAULT => IntAluOp::Shift(ShiftOp::Sll),
            _ if d.funct7 >> 1 == SLLI_UW_FUNCT6 => {
                IntAluOp::Bit(BitOp::ShiftAdd(ShiftAddOp::SlliUw))
            }
            _ => return None,
        },
        i_funct3::SRL_SRA => match d.funct7 {
            i_funct7::DEFAULT => IntAluOp::Shift(ShiftOp::Srl),
            i_funct7::SRA => IntAluOp::Shift(ShiftOp::Sra),
            b_funct7::ROTATE_RIGHT => IntAluOp::Bit(BitOp::Rotate(RotateOp::Ror)),
            _ => return None,
        },
        _ => return None,
    })
}

/// The integer operation an OP word encodes, or `None` for a reserved one.
const fn op(d: &Decoded) -> Option<IntAluOp> {
    Some(match (d.funct7, d.funct3) {
        (i_funct7::DEFAULT, i_funct3::ADD_SUB) => IntAluOp::Arith(ArithOp::Add),
        (i_funct7::SUB, i_funct3::ADD_SUB) => IntAluOp::Arith(ArithOp::Sub),
        (i_funct7::DEFAULT, i_funct3::SLL) => IntAluOp::Shift(ShiftOp::Sll),
        (i_funct7::DEFAULT, i_funct3::SLT) => IntAluOp::Logic(LogicOp::Slt),
        (i_funct7::DEFAULT, i_funct3::SLTU) => IntAluOp::Logic(LogicOp::Sltu),
        (i_funct7::DEFAULT, i_funct3::XOR) => IntAluOp::Logic(LogicOp::Xor),
        (i_funct7::DEFAULT, i_funct3::SRL_SRA) => IntAluOp::Shift(ShiftOp::Srl),
        (i_funct7::SRA, i_funct3::SRL_SRA) => IntAluOp::Shift(ShiftOp::Sra),
        (i_funct7::DEFAULT, i_funct3::OR) => IntAluOp::Logic(LogicOp::Or),
        (i_funct7::DEFAULT, i_funct3::AND) => IntAluOp::Logic(LogicOp::And),
        (m_opcodes::M_EXTENSION, m_funct3::MUL) => IntAluOp::Mul(MulOp::Mul),
        (m_opcodes::M_EXTENSION, m_funct3::MULH) => IntAluOp::Mul(MulOp::Mulh),
        (m_opcodes::M_EXTENSION, m_funct3::MULHSU) => IntAluOp::Mul(MulOp::Mulhsu),
        (m_opcodes::M_EXTENSION, m_funct3::MULHU) => IntAluOp::Mul(MulOp::Mulhu),
        (m_opcodes::M_EXTENSION, m_funct3::DIV) => IntAluOp::Div(DivOp::Div),
        (m_opcodes::M_EXTENSION, m_funct3::DIVU) => IntAluOp::Div(DivOp::Divu),
        (m_opcodes::M_EXTENSION, m_funct3::REM) => IntAluOp::Div(DivOp::Rem),
        (m_opcodes::M_EXTENSION, m_funct3::REMU) => IntAluOp::Div(DivOp::Remu),
        (b_funct7::SH_ADD, b_funct3::SH1ADD) => IntAluOp::Bit(BitOp::ShiftAdd(ShiftAddOp::Sh1Add)),
        (b_funct7::SH_ADD, b_funct3::SH2ADD) => IntAluOp::Bit(BitOp::ShiftAdd(ShiftAddOp::Sh2Add)),
        (b_funct7::SH_ADD, b_funct3::SH3ADD) => IntAluOp::Bit(BitOp::ShiftAdd(ShiftAddOp::Sh3Add)),
        (b_funct7::LOGICAL_NEG, b_funct3::ANDN) => IntAluOp::Bit(BitOp::Andn),
        (b_funct7::LOGICAL_NEG, b_funct3::ORN) => IntAluOp::Bit(BitOp::Orn),
        (b_funct7::LOGICAL_NEG, b_funct3::XNOR) => IntAluOp::Bit(BitOp::Xnor),
        (b_funct7::MIN_MAX, b_funct3::MAX) => IntAluOp::Bit(BitOp::MinMax(MinMaxOp::Max)),
        (b_funct7::MIN_MAX, b_funct3::MAXU) => IntAluOp::Bit(BitOp::MinMax(MinMaxOp::Maxu)),
        (b_funct7::MIN_MAX, b_funct3::MIN) => IntAluOp::Bit(BitOp::MinMax(MinMaxOp::Min)),
        (b_funct7::MIN_MAX, b_funct3::MINU) => IntAluOp::Bit(BitOp::MinMax(MinMaxOp::Minu)),
        (b_funct7::ROTATE, b_funct3::ROL) => IntAluOp::Bit(BitOp::Rotate(RotateOp::Rol)),
        (b_funct7::ROTATE_RIGHT, b_funct3::ROR) => IntAluOp::Bit(BitOp::Rotate(RotateOp::Ror)),
        (b_funct7::CLMUL, b_funct3::CLMUL) => IntAluOp::Bit(BitOp::Clmul(ClmulOp::Clmul)),
        (b_funct7::CLMUL, b_funct3::CLMULH) => IntAluOp::Bit(BitOp::Clmul(ClmulOp::Clmulh)),
        (b_funct7::CLMUL, b_funct3::CLMULR) => IntAluOp::Bit(BitOp::Clmul(ClmulOp::Clmulr)),
        (b_funct7::BCLR, b_funct3::BCLR) => IntAluOp::Bit(BitOp::SingleBit(SingleBitOp::Bclr)),
        (b_funct7::BEXT, b_funct3::BEXT) => IntAluOp::Bit(BitOp::SingleBit(SingleBitOp::Bext)),
        (b_funct7::BINV, b_funct3::BINV) => IntAluOp::Bit(BitOp::SingleBit(SingleBitOp::Binv)),
        (b_funct7::BSET, b_funct3::BSET) => IntAluOp::Bit(BitOp::SingleBit(SingleBitOp::Bset)),
        (b_funct7::PACK, b_funct3::PACK) => IntAluOp::Bit(BitOp::Pack(PackOp::Pack)),
        (b_funct7::PACK, b_funct3::PACKH) => IntAluOp::Bit(BitOp::Pack(PackOp::Packh)),
        (b_funct7::XPERM, b_funct3::XPERM4) => IntAluOp::Bit(BitOp::Xperm4),
        (b_funct7::XPERM, b_funct3::XPERM8) => IntAluOp::Bit(BitOp::Xperm8),
        _ => return None,
    })
}

/// The integer operation an OP-32 word encodes, or `None` for a reserved one.
const fn op_32(d: &Decoded) -> Option<IntAluOp> {
    Some(match (d.funct7, d.funct3) {
        (i_funct7::DEFAULT, i_funct3::ADD_SUB) => IntAluOp::Arith(ArithOp::Add),
        (i_funct7::SUB, i_funct3::ADD_SUB) => IntAluOp::Arith(ArithOp::Sub),
        (i_funct7::DEFAULT, i_funct3::SLL) => IntAluOp::Shift(ShiftOp::Sll),
        (i_funct7::DEFAULT, i_funct3::SRL_SRA) => IntAluOp::Shift(ShiftOp::Srl),
        (i_funct7::SRA, i_funct3::SRL_SRA) => IntAluOp::Shift(ShiftOp::Sra),
        (m_opcodes::M_EXTENSION, m_funct3::MUL) => IntAluOp::Mul(MulOp::Mul),
        (m_opcodes::M_EXTENSION, m_funct3::DIV) => IntAluOp::Div(DivOp::Div),
        (m_opcodes::M_EXTENSION, m_funct3::DIVU) => IntAluOp::Div(DivOp::Divu),
        (m_opcodes::M_EXTENSION, m_funct3::REM) => IntAluOp::Div(DivOp::Rem),
        (m_opcodes::M_EXTENSION, m_funct3::REMU) => IntAluOp::Div(DivOp::Remu),
        (b_funct7::ADD_UW, b_funct3::ADD_UW) => IntAluOp::Bit(BitOp::ShiftAdd(ShiftAddOp::AddUw)),
        (b_funct7::SH_ADD, b_funct3::SH1ADD_UW) => {
            IntAluOp::Bit(BitOp::ShiftAdd(ShiftAddOp::Sh1AddUw))
        }
        (b_funct7::SH_ADD, b_funct3::SH2ADD_UW) => {
            IntAluOp::Bit(BitOp::ShiftAdd(ShiftAddOp::Sh2AddUw))
        }
        (b_funct7::SH_ADD, b_funct3::SH3ADD_UW) => {
            IntAluOp::Bit(BitOp::ShiftAdd(ShiftAddOp::Sh3AddUw))
        }
        (b_funct7::ROTATE, b_funct3::ROL) => IntAluOp::Bit(BitOp::Rotate(RotateOp::Rol)),
        (b_funct7::ROTATE_RIGHT, b_funct3::ROR) => IntAluOp::Bit(BitOp::Rotate(RotateOp::Ror)),
        (b_funct7::PACK, b_funct3::PACKW) => IntAluOp::Bit(BitOp::Pack(PackOp::Packw)),
        _ => return None,
    })
}

/// The rounding `d`'s rounding-mode field selects; a reserved mode makes
/// the instruction illegal.
const fn rounding(inst: u32, d: &Decoded) -> Result<Rounding, Trap> {
    match Rounding::from_field(d.funct3) {
        Some(rounding) => Ok(rounding),
        None => Err(Trap::IllegalInstruction(inst)),
    }
}

/// True for the OP-FP operations whose funct3 is a rounding mode.
const fn rounds(op: FpOp) -> bool {
    matches!(
        op,
        FpOp::Arith(_) | FpOp::Sqrt | FpOp::ToInt(..) | FpOp::FromInt(..) | FpOp::FromFloat(_)
    )
}
