//! Scalar instruction decoding.

use super::vector;
use super::{VEC_WIDTH_8, VEC_WIDTH_16, VEC_WIDTH_32, VEC_WIDTH_64};
use crate::exec::signals::{ControlFlow, ControlSignals, OpASrc, OpBSrc};
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
use crate::isa::fp::{FpFormat, Rounding};
use crate::isa::instruction::{Decoded, InstructionBits};
use crate::isa::op::{AluOp, AtomicOp, CsrOp, IntWidth, MemWidth, OperandFormat, SystemOp};
use crate::isa::privileged::Trap;

/// JALR's only defined funct3.
const JALR_FUNCT3: u32 = 0b000;

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

/// Decodes a scalar instruction (RV64IMAFDC, Zb*, Zk*, Zicsr, Zicbo*,
/// Zfh) into `c`.
pub(super) fn decode(c: &mut ControlSignals, inst: u32, pc: u64, d: &Decoded) -> Result<(), Trap> {
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
            c.control_flow = ControlFlow::Jump;
        }
        i_opcodes::OP_JALR => {
            if d.funct3 != JALR_FUNCT3 {
                return Err(Trap::IllegalInstruction(inst));
            }
            c.reg_write = true;
            c.control_flow = ControlFlow::Jump;
            c.alu = AluOp::Add;
        }
        i_opcodes::OP_BRANCH => {
            if !matches!(
                d.funct3,
                i_funct3::BEQ
                    | i_funct3::BNE
                    | i_funct3::BLT
                    | i_funct3::BGE
                    | i_funct3::BLTU
                    | i_funct3::BGEU
            ) {
                return Err(Trap::IllegalInstruction(inst));
            }
            c.control_flow = ControlFlow::Branch;
            c.b_src = OpBSrc::Reg2;
        }
        i_opcodes::OP_LOAD => {
            c.reg_write = true;
            c.mem_read = true;
            c.alu = AluOp::Add;
            let (w, s) = match d.funct3 {
                i_funct3::LB => (MemWidth::Byte, true),
                i_funct3::LH => (MemWidth::Half, true),
                i_funct3::LW => (MemWidth::Word, true),
                i_funct3::LD => (MemWidth::Double, true),
                i_funct3::LBU => (MemWidth::Byte, false),
                i_funct3::LHU => (MemWidth::Half, false),
                i_funct3::LWU => (MemWidth::Word, false),
                _ => return Err(Trap::IllegalInstruction(inst)),
            };
            c.width = w;
            c.signed_load = s;
        }
        i_opcodes::OP_STORE => {
            c.mem_write = true;
            c.b_src = OpBSrc::Imm;
            c.alu = AluOp::Add;
            c.width = match d.funct3 {
                i_funct3::SB => MemWidth::Byte,
                i_funct3::SH => MemWidth::Half,
                i_funct3::SW => MemWidth::Word,
                i_funct3::SD => MemWidth::Double,
                _ => return Err(Trap::IllegalInstruction(inst)),
            };
        }
        i_opcodes::OP_IMM => {
            c.reg_write = true;
            c.alu = op_imm(inst, d).ok_or(Trap::IllegalInstruction(inst))?;
        }
        i_opcodes::OP_IMM_32 => {
            c.reg_write = true;
            c.alu = op_imm_32(inst, d).ok_or(Trap::IllegalInstruction(inst))?;
            c.format = OperandFormat::Int(word_op_width(c.alu));
        }
        i_opcodes::OP_REG => {
            c.reg_write = true;
            c.b_src = OpBSrc::Reg2;
            c.alu = op(d).ok_or(Trap::IllegalInstruction(inst))?;
        }
        i_opcodes::OP_REG_32 => {
            c.reg_write = true;
            c.b_src = OpBSrc::Reg2;
            c.alu = op_32(d).ok_or(Trap::IllegalInstruction(inst))?;
            c.format = OperandFormat::Int(word_op_width(c.alu));
        }
        a_opcodes::OP_AMO => {
            c.width = match d.funct3 {
                a_funct3::WIDTH_32 => MemWidth::Word,
                a_funct3::WIDTH_64 => MemWidth::Double,
                _ => return Err(Trap::IllegalInstruction(inst)),
            };

            let f5 = d.funct7 >> 2;
            c.atomic_op = Some(match f5 {
                a_funct5::LR => AtomicOp::Lr,
                a_funct5::SC => AtomicOp::Sc,
                a_funct5::AMOSWAP => AtomicOp::Swap,
                a_funct5::AMOADD => AtomicOp::Add,
                a_funct5::AMOXOR => AtomicOp::Xor,
                a_funct5::AMOAND => AtomicOp::And,
                a_funct5::AMOOR => AtomicOp::Or,
                a_funct5::AMOMIN => AtomicOp::Min,
                a_funct5::AMOMAX => AtomicOp::Max,
                a_funct5::AMOMINU => AtomicOp::MinU,
                a_funct5::AMOMAXU => AtomicOp::MaxU,
                _ => return Err(Trap::IllegalInstruction(inst)),
            });

            if c.atomic_op == Some(AtomicOp::Lr) && !d.rs2.is_zero() {
                return Err(Trap::IllegalInstruction(inst));
            }
            c.alu = AluOp::Add;
            c.a_src = OpASrc::Reg1;
            c.b_src = OpBSrc::Zero;
            c.acquire = inst & AMO_AQ != 0;
            c.release = inst & AMO_RL != 0;
            c.mem_read = true;
            c.mem_write = c.atomic_op != Some(AtomicOp::Lr);
            c.reg_write = true;
            // AMO and LR always sign-extend the loaded old value. SC writes
            // a 0/1 success code and overrides load_data in memory2.
            c.signed_load = true;
        }
        f_opcodes::OP_LOAD_FP => match d.funct3 {
            FP_WIDTH_HALF => {
                c.fp_reg_write = true;
                c.mem_read = true;
                c.alu = AluOp::Add;
                c.width = MemWidth::Half;
                c.format = OperandFormat::Fp(FpFormat::Half);
            }
            FP_WIDTH_WORD => {
                c.fp_reg_write = true;
                c.mem_read = true;
                c.alu = AluOp::Add;
                c.width = MemWidth::Word;
                c.format = OperandFormat::Fp(FpFormat::Single);
            }
            FP_WIDTH_DOUBLE => {
                c.fp_reg_write = true;
                c.mem_read = true;
                c.alu = AluOp::Add;
                c.width = MemWidth::Double;
                c.format = OperandFormat::Fp(FpFormat::Double);
            }
            VEC_WIDTH_8 | VEC_WIDTH_16 | VEC_WIDTH_32 | VEC_WIDTH_64 => {
                vector::decode_vec_load(inst, d.funct3, c)?;
            }
            _ => return Err(Trap::IllegalInstruction(inst)),
        },
        f_opcodes::OP_STORE_FP => match d.funct3 {
            FP_WIDTH_HALF => {
                c.mem_write = true;
                c.rs1_fp = false;
                c.rs2_fp = true;
                c.b_src = OpBSrc::Imm;
                c.alu = AluOp::Add;
                c.width = MemWidth::Half;
                c.format = OperandFormat::Fp(FpFormat::Half);
            }
            FP_WIDTH_WORD => {
                c.mem_write = true;
                c.rs1_fp = false;
                c.rs2_fp = true;
                c.b_src = OpBSrc::Imm;
                c.alu = AluOp::Add;
                c.width = MemWidth::Word;
                c.format = OperandFormat::Fp(FpFormat::Single);
            }
            FP_WIDTH_DOUBLE => {
                c.mem_write = true;
                c.rs1_fp = false;
                c.rs2_fp = true;
                c.b_src = OpBSrc::Imm;
                c.alu = AluOp::Add;
                c.width = MemWidth::Double;
                c.format = OperandFormat::Fp(FpFormat::Double);
            }
            VEC_WIDTH_8 | VEC_WIDTH_16 | VEC_WIDTH_32 | VEC_WIDTH_64 => {
                vector::decode_vec_store(inst, d.funct3, c)?;
            }
            _ => return Err(Trap::IllegalInstruction(inst)),
        },
        f_opcodes::OP_FP => {
            c.format =
                OperandFormat::Fp(fp_format(d.funct7).ok_or(Trap::IllegalInstruction(inst))?);

            c.rs1_fp = true;
            c.rs2_fp = true;
            c.fp_reg_write = true;
            c.b_src = OpBSrc::Reg2;

            c.alu = match d.funct7 {
                f_funct7::FADD | d_funct7::FADD_D | h_funct7::FADD_H => AluOp::FAdd,
                f_funct7::FSUB | d_funct7::FSUB_D | h_funct7::FSUB_H => AluOp::FSub,
                f_funct7::FMUL | d_funct7::FMUL_D | h_funct7::FMUL_H => AluOp::FMul,
                f_funct7::FDIV | d_funct7::FDIV_D | h_funct7::FDIV_H => AluOp::FDiv,
                f_funct7::FSQRT | d_funct7::FSQRT_D | h_funct7::FSQRT_H if d.rs2.is_zero() => {
                    AluOp::FSqrt
                }
                f_funct7::FSGNJ | d_funct7::FSGNJ_D | h_funct7::FSGNJ_H => match d.funct3 {
                    f_funct3::FSGNJ => AluOp::FSgnJ,
                    f_funct3::FSGNJN => AluOp::FSgnJN,
                    f_funct3::FSGNJX => AluOp::FSgnJX,
                    _ => return Err(Trap::IllegalInstruction(inst)),
                },
                f_funct7::FMIN_MAX | d_funct7::FMIN_MAX_D | h_funct7::FMIN_MAX_H => {
                    match d.funct3 {
                        f_funct3::FMIN => AluOp::FMin,
                        f_funct3::FMAX => AluOp::FMax,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                f_funct7::FCMP | d_funct7::FCMP_D | h_funct7::FCMP_H => {
                    c.fp_reg_write = false;
                    c.reg_write = true;
                    match d.funct3 {
                        f_funct3::FEQ => AluOp::FEq,
                        f_funct3::FLT => AluOp::FLt,
                        f_funct3::FLE => AluOp::FLe,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                f_funct7::FCLASS_MV_X_F | d_funct7::FCLASS_MV_X_D | h_funct7::FCLASS_MV_X_H
                    if d.rs2.is_zero() =>
                {
                    c.fp_reg_write = false;
                    c.reg_write = true;
                    c.rs1_fp = true;
                    match d.funct3 {
                        f_funct3::FMV_X_W => AluOp::FMvToX,
                        f_funct3::FCLASS => AluOp::FClass,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                f_funct7::FMV_F_X | d_funct7::FMV_D_X | h_funct7::FMV_H_X
                    if d.rs2.is_zero() && d.funct3 == f_funct3::FMV_X_W =>
                {
                    c.rs1_fp = false;
                    c.fp_reg_write = true;
                    c.a_src = OpASrc::Reg1;
                    AluOp::FMvToF
                }
                f_funct7::FCVT_W_F | d_funct7::FCVT_W_D | h_funct7::FCVT_W_H => {
                    c.fp_reg_write = false;
                    c.reg_write = true;
                    c.rs1_fp = true;
                    match d.rs2.as_u8() {
                        0 => AluOp::FCvtWS,
                        1 => AluOp::FCvtWUS,
                        2 => AluOp::FCvtLS,
                        3 => AluOp::FCvtLUS,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                f_funct7::FCVT_F_W | d_funct7::FCVT_D_W | h_funct7::FCVT_H_W => {
                    c.rs1_fp = false;
                    c.fp_reg_write = true;
                    c.a_src = OpASrc::Reg1;
                    match d.rs2.as_u8() {
                        0 => AluOp::FCvtSW,
                        1 => AluOp::FCvtSWU,
                        2 => AluOp::FCvtSL,
                        3 => AluOp::FCvtSLU,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                // FP↔FP conversion: target fmt in funct7[1:0], source fmt in rs2[1:0].
                f_funct7::FCVT_DS => {
                    // Target = double (fmt=01). Source selected by rs2.
                    c.format = OperandFormat::Fp(FpFormat::Double);
                    match d.rs2.as_u8() {
                        0 => AluOp::FCvtDS, // fcvt.d.s (source single)
                        2 => AluOp::FCvtDH, // fcvt.d.h (source half)
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                d_funct7::FCVT_S_D => {
                    // Target = single (fmt=00). Source selected by rs2.
                    c.format = OperandFormat::Fp(FpFormat::Single);
                    match d.rs2.as_u8() {
                        1 => AluOp::FCvtSD, // fcvt.s.d (source double)
                        2 => AluOp::FCvtSH, // fcvt.s.h (source half)
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                h_funct7::FCVT_H_FP => {
                    // Target = half (fmt=10). Source selected by rs2.
                    c.format = OperandFormat::Fp(FpFormat::Half);
                    match d.rs2.as_u8() {
                        0 => AluOp::FCvtHS, // fcvt.h.s (source single)
                        1 => AluOp::FCvtHD, // fcvt.h.d (source double)
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                _ => return Err(Trap::IllegalInstruction(inst)),
            };
            if rounds(c.alu) {
                c.fp_rm = Some(rounding(inst, d)?);
            }
        }
        d_opcodes::OP_FMADD | d_opcodes::OP_FMSUB | d_opcodes::OP_FNMADD | d_opcodes::OP_FNMSUB => {
            c.rs1_fp = true;
            c.rs2_fp = true;
            c.rs3_fp = true;
            c.fp_reg_write = true;
            c.b_src = OpBSrc::Reg2;
            c.format =
                OperandFormat::Fp(fp_format(d.funct7).ok_or(Trap::IllegalInstruction(inst))?);
            c.fp_rm = Some(rounding(inst, d)?);

            c.alu = match d.opcode {
                d_opcodes::OP_FMADD => AluOp::FMAdd,
                d_opcodes::OP_FMSUB => AluOp::FMSub,
                d_opcodes::OP_FNMADD => AluOp::FNMAdd,
                d_opcodes::OP_FNMSUB => AluOp::FNMSub,
                _ => AluOp::Add,
            };
        }
        sys_ops::OP_SYSTEM => {
            // SFENCE.VMA is R-type: funct7=0x09, rs2, rs1, funct3=0, rd=0.
            // Mask out rs1 (bits 19:15) and rs2 (bits 24:20) for matching.
            if (inst & 0xFE007FFF) == sys_ops::SFENCE_VMA {
                c.system_op = SystemOp::SfenceVma;
            } else if d.funct3 == 0 {
                c.system_op = match d.raw {
                    sys_ops::EBREAK => return Err(Trap::Breakpoint(pc)),
                    sys_ops::MRET => SystemOp::Mret,
                    sys_ops::SRET => SystemOp::Sret,
                    sys_ops::WFI => SystemOp::Wfi,
                    sys_ops::ECALL => SystemOp::Ecall,
                    _ => return Err(Trap::IllegalInstruction(inst)),
                };
            } else {
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
                c.a_src = OpASrc::Reg1;
                c.b_src = OpBSrc::Zero;
                c.reg_write = !d.rd.is_zero();
            }
        }
        sys_ops::OP_PSEUDO => c.system_op = SystemOp::Pseudo,
        i_opcodes::OP_MISC_MEM => match d.funct3 {
            i_funct3::FENCE => c.system_op = SystemOp::Fence,
            i_funct3::FENCE_I => c.system_op = SystemOp::FenceI,
            i_funct3::CBO => {
                if !d.rd.is_zero() {
                    return Err(Trap::IllegalInstruction(inst));
                }
                match d.imm {
                    zicboz::CBO_ZERO_IMM => c.system_op = SystemOp::CboZero,
                    zicboz::CBO_INVAL_IMM => c.system_op = SystemOp::CboInval,
                    zicboz::CBO_CLEAN_IMM => c.system_op = SystemOp::CboClean,
                    zicboz::CBO_FLUSH_IMM => c.system_op = SystemOp::CboFlush,
                    _ => return Err(Trap::IllegalInstruction(inst)),
                }
            }
            _ => return Err(Trap::IllegalInstruction(inst)),
        },
        _ => return Err(Trap::IllegalInstruction(inst)),
    }
    Ok(())
}

/// The 12-bit I-type immediate, which selects the Zbb/Zbkb unary ops.
const fn imm12(inst: u32) -> u32 {
    (inst >> b_funct3::I_IMM_SHIFT) & 0xFFF
}

/// The width a W-opcode operation computes at: Zba's `.uw` ops sit in the
/// W opcodes but produce a full 64-bit result.
const fn word_op_width(op: AluOp) -> IntWidth {
    match op {
        AluOp::AddUw | AluOp::Sh1AddUw | AluOp::Sh2AddUw | AluOp::Sh3AddUw | AluOp::SlliUw => {
            IntWidth::Double
        }
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

/// The operation an OP-IMM word encodes, or `None` for a reserved one.
const fn op_imm(inst: u32, d: &Decoded) -> Option<AluOp> {
    let funct6 = d.funct7 >> 1;
    Some(match d.funct3 {
        i_funct3::ADD_SUB => AluOp::Add,
        i_funct3::SLT => AluOp::Slt,
        i_funct3::SLTU => AluOp::Sltu,
        i_funct3::XOR => AluOp::Xor,
        i_funct3::OR => AluOp::Or,
        i_funct3::AND => AluOp::And,
        i_funct3::SLL => match imm12(inst) {
            b_funct3::CLZ_IMM => AluOp::Clz,
            b_funct3::CTZ_IMM => AluOp::Ctz,
            b_funct3::CPOP_IMM => AluOp::Cpop,
            b_funct3::SEXT_B_IMM => AluOp::SextB,
            b_funct3::SEXT_H_IMM => AluOp::SextH,
            _ => match funct6 {
                SLLI_SRLI_FUNCT6 => AluOp::Sll,
                BCLRI_FUNCT6 => AluOp::Bclr,
                BINVI_FUNCT6 => AluOp::Binv,
                BSETI_FUNCT6 => AluOp::Bset,
                _ => return None,
            },
        },
        i_funct3::SRL_SRA => match imm12(inst) {
            b_funct3::ORC_B_IMM => AluOp::OrcB,
            b_funct3::REV8_IMM => AluOp::Rev8,
            b_funct3::BREV8_IMM => AluOp::Brev8,
            _ => match funct6 {
                SLLI_SRLI_FUNCT6 => AluOp::Srl,
                SRAI_FUNCT6 => AluOp::Sra,
                RORI_FUNCT6 => AluOp::Ror,
                BEXTI_FUNCT6 => AluOp::Bext,
                _ => return None,
            },
        },
        _ => return None,
    })
}

/// The operation an OP-IMM-32 word encodes, or `None` for a reserved one.
const fn op_imm_32(inst: u32, d: &Decoded) -> Option<AluOp> {
    Some(match d.funct3 {
        i_funct3::ADD_SUB => AluOp::Add,
        i_funct3::SLL => match imm12(inst) {
            b_funct3::CLZ_IMM => AluOp::Clz,
            b_funct3::CTZ_IMM => AluOp::Ctz,
            b_funct3::CPOP_IMM => AluOp::Cpop,
            _ if d.funct7 == i_funct7::DEFAULT => AluOp::Sll,
            _ if d.funct7 >> 1 == SLLI_UW_FUNCT6 => AluOp::SlliUw,
            _ => return None,
        },
        i_funct3::SRL_SRA => match d.funct7 {
            i_funct7::DEFAULT => AluOp::Srl,
            i_funct7::SRA => AluOp::Sra,
            b_funct7::ROTATE_RIGHT => AluOp::Ror,
            _ => return None,
        },
        _ => return None,
    })
}

/// The operation an OP word encodes, or `None` for a reserved one.
const fn op(d: &Decoded) -> Option<AluOp> {
    Some(match (d.funct7, d.funct3) {
        (i_funct7::DEFAULT, i_funct3::ADD_SUB) => AluOp::Add,
        (i_funct7::SUB, i_funct3::ADD_SUB) => AluOp::Sub,
        (i_funct7::DEFAULT, i_funct3::SLL) => AluOp::Sll,
        (i_funct7::DEFAULT, i_funct3::SLT) => AluOp::Slt,
        (i_funct7::DEFAULT, i_funct3::SLTU) => AluOp::Sltu,
        (i_funct7::DEFAULT, i_funct3::XOR) => AluOp::Xor,
        (i_funct7::DEFAULT, i_funct3::SRL_SRA) => AluOp::Srl,
        (i_funct7::SRA, i_funct3::SRL_SRA) => AluOp::Sra,
        (i_funct7::DEFAULT, i_funct3::OR) => AluOp::Or,
        (i_funct7::DEFAULT, i_funct3::AND) => AluOp::And,
        (m_opcodes::M_EXTENSION, m_funct3::MUL) => AluOp::Mul,
        (m_opcodes::M_EXTENSION, m_funct3::MULH) => AluOp::Mulh,
        (m_opcodes::M_EXTENSION, m_funct3::MULHSU) => AluOp::Mulhsu,
        (m_opcodes::M_EXTENSION, m_funct3::MULHU) => AluOp::Mulhu,
        (m_opcodes::M_EXTENSION, m_funct3::DIV) => AluOp::Div,
        (m_opcodes::M_EXTENSION, m_funct3::DIVU) => AluOp::Divu,
        (m_opcodes::M_EXTENSION, m_funct3::REM) => AluOp::Rem,
        (m_opcodes::M_EXTENSION, m_funct3::REMU) => AluOp::Remu,
        (b_funct7::SH_ADD, b_funct3::SH1ADD) => AluOp::Sh1Add,
        (b_funct7::SH_ADD, b_funct3::SH2ADD) => AluOp::Sh2Add,
        (b_funct7::SH_ADD, b_funct3::SH3ADD) => AluOp::Sh3Add,
        (b_funct7::LOGICAL_NEG, b_funct3::ANDN) => AluOp::Andn,
        (b_funct7::LOGICAL_NEG, b_funct3::ORN) => AluOp::Orn,
        (b_funct7::LOGICAL_NEG, b_funct3::XNOR) => AluOp::Xnor,
        (b_funct7::MIN_MAX, b_funct3::MAX) => AluOp::Max,
        (b_funct7::MIN_MAX, b_funct3::MAXU) => AluOp::Maxu,
        (b_funct7::MIN_MAX, b_funct3::MIN) => AluOp::Min,
        (b_funct7::MIN_MAX, b_funct3::MINU) => AluOp::Minu,
        (b_funct7::ROTATE, b_funct3::ROL) => AluOp::Rol,
        (b_funct7::ROTATE_RIGHT, b_funct3::ROR) => AluOp::Ror,
        (b_funct7::CLMUL, b_funct3::CLMUL) => AluOp::Clmul,
        (b_funct7::CLMUL, b_funct3::CLMULH) => AluOp::Clmulh,
        (b_funct7::CLMUL, b_funct3::CLMULR) => AluOp::Clmulr,
        (b_funct7::BCLR, b_funct3::BCLR) => AluOp::Bclr,
        (b_funct7::BEXT, b_funct3::BEXT) => AluOp::Bext,
        (b_funct7::BINV, b_funct3::BINV) => AluOp::Binv,
        (b_funct7::BSET, b_funct3::BSET) => AluOp::Bset,
        (b_funct7::PACK, b_funct3::PACK) => AluOp::Pack,
        (b_funct7::PACK, b_funct3::PACKH) => AluOp::Packh,
        (b_funct7::XPERM, b_funct3::XPERM4) => AluOp::Xperm4,
        (b_funct7::XPERM, b_funct3::XPERM8) => AluOp::Xperm8,
        _ => return None,
    })
}

/// The operation an OP-32 word encodes, or `None` for a reserved one.
const fn op_32(d: &Decoded) -> Option<AluOp> {
    Some(match (d.funct7, d.funct3) {
        (i_funct7::DEFAULT, i_funct3::ADD_SUB) => AluOp::Add,
        (i_funct7::SUB, i_funct3::ADD_SUB) => AluOp::Sub,
        (i_funct7::DEFAULT, i_funct3::SLL) => AluOp::Sll,
        (i_funct7::DEFAULT, i_funct3::SRL_SRA) => AluOp::Srl,
        (i_funct7::SRA, i_funct3::SRL_SRA) => AluOp::Sra,
        (m_opcodes::M_EXTENSION, m_funct3::MUL) => AluOp::Mul,
        (m_opcodes::M_EXTENSION, m_funct3::DIV) => AluOp::Div,
        (m_opcodes::M_EXTENSION, m_funct3::DIVU) => AluOp::Divu,
        (m_opcodes::M_EXTENSION, m_funct3::REM) => AluOp::Rem,
        (m_opcodes::M_EXTENSION, m_funct3::REMU) => AluOp::Remu,
        (b_funct7::ADD_UW, b_funct3::ADD_UW) => AluOp::AddUw,
        (b_funct7::SH_ADD, b_funct3::SH1ADD_UW) => AluOp::Sh1AddUw,
        (b_funct7::SH_ADD, b_funct3::SH2ADD_UW) => AluOp::Sh2AddUw,
        (b_funct7::SH_ADD, b_funct3::SH3ADD_UW) => AluOp::Sh3AddUw,
        (b_funct7::ROTATE, b_funct3::ROL) => AluOp::Rol,
        (b_funct7::ROTATE_RIGHT, b_funct3::ROR) => AluOp::Ror,
        (b_funct7::PACK, b_funct3::PACKW) => AluOp::Packw,
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
const fn rounds(op: AluOp) -> bool {
    matches!(
        op,
        AluOp::FAdd
            | AluOp::FSub
            | AluOp::FMul
            | AluOp::FDiv
            | AluOp::FSqrt
            | AluOp::FCvtWS
            | AluOp::FCvtWUS
            | AluOp::FCvtLS
            | AluOp::FCvtLUS
            | AluOp::FCvtSW
            | AluOp::FCvtSWU
            | AluOp::FCvtSL
            | AluOp::FCvtSLU
            | AluOp::FCvtDS
            | AluOp::FCvtDH
            | AluOp::FCvtSD
            | AluOp::FCvtSH
            | AluOp::FCvtHS
            | AluOp::FCvtHD
    )
}
