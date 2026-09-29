//! Scalar instruction decoding.

use super::vector;
use super::{VEC_WIDTH_8, VEC_WIDTH_16, VEC_WIDTH_32, VEC_WIDTH_64};
use crate::common::error::Trap;
use crate::core::exec::signals::{
    AluOp, AtomicOp, ControlFlow, ControlSignals, CsrOp, MemWidth, OpASrc, OpBSrc, SystemOp,
};
use crate::core::units::fpu::rounding_modes::RoundingMode;
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
use crate::isa::instruction::{Decoded, InstructionBits};

/// Bit 5 of funct7 field indicating alternate encoding (e.g., SUB vs ADD).
const FUNCT7_ALT_BIT: u32 = 0x20;

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
            c.reg_write = true;
            c.control_flow = ControlFlow::Jump;
            c.alu = AluOp::Add;
        }
        i_opcodes::OP_BRANCH => {
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
        i_opcodes::OP_IMM | i_opcodes::OP_IMM_32 => {
            c.reg_write = true;
            c.is_rv32 = d.opcode == i_opcodes::OP_IMM_32;
            c.alu = match d.funct3 {
                i_funct3::ADD_SUB => AluOp::Add,
                i_funct3::SLT => AluOp::Slt,
                i_funct3::SLTU => AluOp::Sltu,
                i_funct3::XOR => AluOp::Xor,
                i_funct3::OR => AluOp::Or,
                i_funct3::AND => AluOp::And,
                i_funct3::SLL => {
                    // Shift-like encodings use funct7 to select the operation.
                    // B-extension unary ops (clz, ctz, cpop, sext.*) encode
                    // the operation in the full imm[11:0] field.
                    let imm12 = (inst >> b_funct3::I_IMM_SHIFT) & 0xFFF;
                    let top6 = d.funct7 >> 1; // bits 31:26
                    if d.opcode == i_opcodes::OP_IMM {
                        match imm12 {
                            // Zbb: clz, ctz, cpop, sext.b, sext.h
                            b_funct3::CLZ_IMM => AluOp::Clz,
                            b_funct3::CTZ_IMM => AluOp::Ctz,
                            b_funct3::CPOP_IMM => AluOp::Cpop,
                            b_funct3::SEXT_B_IMM => AluOp::SextB,
                            b_funct3::SEXT_H_IMM => AluOp::SextH,
                            // Zbs: bclri (top 6 bits select, low 6 = shamt)
                            _ if top6 == (b_funct7::BCLR >> 1) => AluOp::Bclr,
                            // Zbs: binvi
                            _ if top6 == (b_funct7::BINV >> 1) => AluOp::Binv,
                            // Zbs: bseti
                            _ if top6 == (b_funct7::BSET >> 1) => AluOp::Bset,
                            // Base I: slli
                            _ => AluOp::Sll,
                        }
                    } else {
                        // OP_IMM_32
                        let imm12_w = (inst >> b_funct3::I_IMM_SHIFT) & 0xFFF;
                        match imm12_w {
                            // Zbb: clzw, ctzw, cpopw
                            b_funct3::CLZ_IMM => AluOp::Clz,
                            b_funct3::CTZ_IMM => AluOp::Ctz,
                            b_funct3::CPOP_IMM => AluOp::Cpop,
                            _ => match top6 {
                                // Zba: slli.uw (produces 64-bit result)
                                _ if top6 == (b_funct7::SLLI_UW >> 1) => {
                                    c.is_rv32 = false;
                                    AluOp::SlliUw
                                }
                                // Base I: slliw
                                _ => AluOp::Sll,
                            },
                        }
                    }
                }
                i_funct3::SRL_SRA => {
                    let imm12 = (inst >> b_funct3::I_IMM_SHIFT) & 0xFFF;
                    let top6 = d.funct7 >> 1; // bits 31:26

                    if d.opcode == i_opcodes::OP_IMM {
                        match imm12 {
                            // Zbb: orc.b, rev8 (full imm[11:0] selects)
                            b_funct3::ORC_B_IMM => AluOp::OrcB,
                            b_funct3::REV8_IMM => AluOp::Rev8,
                            // Zbkb: brev8
                            b_funct3::BREV8_IMM => AluOp::Brev8,
                            // Zbb: rori
                            _ if top6 == (b_funct7::ROTATE_RIGHT >> 1) => AluOp::Ror,
                            // Zbs: bexti
                            _ if top6 == (b_funct7::BEXT >> 1) => AluOp::Bext,
                            // Base I: srai
                            _ if (d.funct7 & FUNCT7_ALT_BIT) != 0 => AluOp::Sra,
                            // Base I: srli
                            _ => AluOp::Srl,
                        }
                    } else {
                        // OP_IMM_32
                        match top6 {
                            // Zbb: roriw
                            _ if top6 == (b_funct7::ROTATE_RIGHT >> 1) => AluOp::Ror,
                            // Base I: sraiw
                            _ if (d.funct7 & FUNCT7_ALT_BIT) != 0 => AluOp::Sra,
                            // Base I: srliw
                            _ => AluOp::Srl,
                        }
                    }
                }
                _ => return Err(Trap::IllegalInstruction(inst)),
            };
        }
        i_opcodes::OP_REG | i_opcodes::OP_REG_32 => {
            c.reg_write = true;
            c.is_rv32 = d.opcode == i_opcodes::OP_REG_32;
            c.b_src = OpBSrc::Reg2;

            if d.funct7 == m_opcodes::M_EXTENSION {
                // M-extension: multiply / divide
                c.alu = match d.funct3 {
                    m_funct3::MUL => AluOp::Mul,
                    m_funct3::MULH => AluOp::Mulh,
                    m_funct3::MULHSU => AluOp::Mulhsu,
                    m_funct3::MULHU => AluOp::Mulhu,
                    m_funct3::DIV => AluOp::Div,
                    m_funct3::DIVU => AluOp::Divu,
                    m_funct3::REM => AluOp::Rem,
                    m_funct3::REMU => AluOp::Remu,
                    _ => return Err(Trap::IllegalInstruction(inst)),
                };
            } else {
                c.alu = match (d.funct3, d.funct7) {
                    // Base I-extension
                    (i_funct3::ADD_SUB, i_funct7::DEFAULT) => AluOp::Add,
                    (i_funct3::ADD_SUB, i_funct7::SUB) => AluOp::Sub,
                    (i_funct3::SLL, i_funct7::DEFAULT) => AluOp::Sll,
                    (i_funct3::SLT, i_funct7::DEFAULT) => AluOp::Slt,
                    (i_funct3::SLTU, i_funct7::DEFAULT) => AluOp::Sltu,
                    (i_funct3::XOR, i_funct7::DEFAULT) => AluOp::Xor,
                    (i_funct3::SRL_SRA, i_funct7::DEFAULT) => AluOp::Srl,
                    (i_funct3::SRL_SRA, i_funct7::SRA) => AluOp::Sra,
                    (i_funct3::OR, i_funct7::DEFAULT) => AluOp::Or,
                    (i_funct3::AND, i_funct7::DEFAULT) => AluOp::And,

                    (b_funct3::SH1ADD, b_funct7::SH_ADD) if d.opcode == i_opcodes::OP_REG => {
                        AluOp::Sh1Add
                    }
                    (b_funct3::SH2ADD, b_funct7::SH_ADD) if d.opcode == i_opcodes::OP_REG => {
                        AluOp::Sh2Add
                    }
                    (b_funct3::SH3ADD, b_funct7::SH_ADD) if d.opcode == i_opcodes::OP_REG => {
                        AluOp::Sh3Add
                    }
                    // add.uw (OP_REG_32 only, but produces a 64-bit result)
                    (b_funct3::ADD_UW, b_funct7::ADD_UW) if d.opcode == i_opcodes::OP_REG_32 => {
                        c.is_rv32 = false;
                        AluOp::AddUw
                    }
                    // sh1add.uw / sh2add.uw / sh3add.uw (OP_REG_32, 64-bit result)
                    (b_funct3::SH1ADD_UW, b_funct7::SH_ADD) if d.opcode == i_opcodes::OP_REG_32 => {
                        c.is_rv32 = false;
                        AluOp::Sh1AddUw
                    }
                    (b_funct3::SH2ADD_UW, b_funct7::SH_ADD) if d.opcode == i_opcodes::OP_REG_32 => {
                        c.is_rv32 = false;
                        AluOp::Sh2AddUw
                    }
                    (b_funct3::SH3ADD_UW, b_funct7::SH_ADD) if d.opcode == i_opcodes::OP_REG_32 => {
                        c.is_rv32 = false;
                        AluOp::Sh3AddUw
                    }

                    (b_funct3::ANDN, b_funct7::LOGICAL_NEG) => AluOp::Andn,
                    (b_funct3::ORN, b_funct7::LOGICAL_NEG) => AluOp::Orn,
                    (b_funct3::XNOR, b_funct7::LOGICAL_NEG) => AluOp::Xnor,
                    (b_funct3::MAX, b_funct7::MIN_MAX) => AluOp::Max,
                    (b_funct3::MAXU, b_funct7::MIN_MAX) => AluOp::Maxu,
                    (b_funct3::MIN, b_funct7::MIN_MAX) => AluOp::Min,
                    (b_funct3::MINU, b_funct7::MIN_MAX) => AluOp::Minu,
                    (b_funct3::ROL, b_funct7::ROTATE) => AluOp::Rol,
                    (b_funct3::ROR, b_funct7::ROTATE_RIGHT) => AluOp::Ror,
                    (b_funct3::CLMUL, b_funct7::CLMUL) if d.opcode == i_opcodes::OP_REG => {
                        AluOp::Clmul
                    }
                    (b_funct3::CLMULH, b_funct7::CLMUL) if d.opcode == i_opcodes::OP_REG => {
                        AluOp::Clmulh
                    }
                    (b_funct3::CLMULR, b_funct7::CLMUL) if d.opcode == i_opcodes::OP_REG => {
                        AluOp::Clmulr
                    }

                    (b_funct3::BCLR, b_funct7::BCLR) => AluOp::Bclr,
                    (b_funct3::BEXT, b_funct7::BEXT) => AluOp::Bext,
                    (b_funct3::BINV, b_funct7::BINV) => AluOp::Binv,
                    (b_funct3::BSET, b_funct7::BSET) => AluOp::Bset,

                    (b_funct3::PACK, b_funct7::PACK) if d.opcode == i_opcodes::OP_REG => {
                        AluOp::Pack
                    }
                    (b_funct3::PACKH, b_funct7::PACK) if d.opcode == i_opcodes::OP_REG => {
                        AluOp::Packh
                    }
                    (b_funct3::PACKW, b_funct7::PACK) if d.opcode == i_opcodes::OP_REG_32 => {
                        AluOp::Packw
                    }

                    (b_funct3::XPERM4, b_funct7::XPERM) if d.opcode == i_opcodes::OP_REG => {
                        AluOp::Xperm4
                    }
                    (b_funct3::XPERM8, b_funct7::XPERM) if d.opcode == i_opcodes::OP_REG => {
                        AluOp::Xperm8
                    }

                    _ => return Err(Trap::IllegalInstruction(inst)),
                };
            }
        }
        a_opcodes::OP_AMO => {
            c.width = match d.funct3 {
                a_funct3::WIDTH_32 => MemWidth::Word,
                a_funct3::WIDTH_64 => MemWidth::Double,
                _ => return Err(Trap::IllegalInstruction(inst)),
            };

            let f5 = d.funct7 >> 2;
            c.atomic_op = match f5 {
                a_funct5::LR => AtomicOp::Lr,
                a_funct5::SC => AtomicOp::Sc,
                a_funct5::AMOSWAP => AtomicOp::Swap,
                a_funct5::AMOADD => AtomicOp::Add,
                a_funct5::AMOXOR => AtomicOp::Xor,
                a_funct5::AMOAND => AtomicOp::And,
                a_funct5::AMOOR => AtomicOp::Or,
                a_funct5::AMOMIN => AtomicOp::Min,
                a_funct5::AMOMAX => AtomicOp::Max,
                a_funct5::AMOMINU => AtomicOp::Minu,
                a_funct5::AMOMAXU => AtomicOp::Maxu,
                _ => return Err(Trap::IllegalInstruction(inst)),
            };

            c.alu = AluOp::Add;
            c.a_src = OpASrc::Reg1;
            c.b_src = OpBSrc::Zero;
            c.acquire = inst & AMO_AQ != 0;
            c.release = inst & AMO_RL != 0;
            c.mem_read = true;
            c.mem_write = c.atomic_op != AtomicOp::Lr;
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
                c.is_f16 = true;
            }
            FP_WIDTH_WORD => {
                c.fp_reg_write = true;
                c.mem_read = true;
                c.alu = AluOp::Add;
                c.width = MemWidth::Word;
            }
            FP_WIDTH_DOUBLE => {
                c.fp_reg_write = true;
                c.mem_read = true;
                c.alu = AluOp::Add;
                c.width = MemWidth::Double;
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
                c.is_f16 = true;
            }
            FP_WIDTH_WORD => {
                c.mem_write = true;
                c.rs1_fp = false;
                c.rs2_fp = true;
                c.b_src = OpBSrc::Imm;
                c.alu = AluOp::Add;
                c.width = MemWidth::Word;
            }
            FP_WIDTH_DOUBLE => {
                c.mem_write = true;
                c.rs1_fp = false;
                c.rs2_fp = true;
                c.b_src = OpBSrc::Imm;
                c.alu = AluOp::Add;
                c.width = MemWidth::Double;
            }
            VEC_WIDTH_8 | VEC_WIDTH_16 | VEC_WIDTH_32 | VEC_WIDTH_64 => {
                vector::decode_vec_store(inst, d.funct3, c)?;
            }
            _ => return Err(Trap::IllegalInstruction(inst)),
        },
        f_opcodes::OP_FP => {
            let fmt = d.funct7 & 0x3;
            c.is_rv32 = fmt == FP_FMT_SINGLE;
            c.is_f16 = fmt == FP_FMT_HALF;
            let is_double = fmt == FP_FMT_DOUBLE;

            if !c.is_rv32 && !is_double && !c.is_f16 {
                return Err(Trap::IllegalInstruction(inst));
            }

            // Decode rounding mode from funct3. 0b111 = dynamic (use fcsr.frm).
            c.fp_rm = RoundingMode::from_bits(d.funct3 as u8);

            c.rs1_fp = true;
            c.rs2_fp = true;
            c.fp_reg_write = true;
            c.b_src = OpBSrc::Reg2;

            c.alu = match d.funct7 {
                f_funct7::FADD | d_funct7::FADD_D | h_funct7::FADD_H => AluOp::FAdd,
                f_funct7::FSUB | d_funct7::FSUB_D | h_funct7::FSUB_H => AluOp::FSub,
                f_funct7::FMUL | d_funct7::FMUL_D | h_funct7::FMUL_H => AluOp::FMul,
                f_funct7::FDIV | d_funct7::FDIV_D | h_funct7::FDIV_H => AluOp::FDiv,
                f_funct7::FSQRT | d_funct7::FSQRT_D | h_funct7::FSQRT_H => AluOp::FSqrt,
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
                f_funct7::FCLASS_MV_X_F | d_funct7::FCLASS_MV_X_D | h_funct7::FCLASS_MV_X_H => {
                    c.fp_reg_write = false;
                    c.reg_write = true;
                    c.rs1_fp = true;
                    match d.funct3 {
                        f_funct3::FMV_X_W => AluOp::FMvToX,
                        f_funct3::FCLASS => AluOp::FClass,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                f_funct7::FMV_F_X | d_funct7::FMV_D_X | h_funct7::FMV_H_X => {
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
                    c.is_rv32 = false;
                    c.is_f16 = false;
                    match d.rs2.as_u8() {
                        0 => AluOp::FCvtDS, // fcvt.d.s (source single)
                        2 => AluOp::FCvtDH, // fcvt.d.h (source half)
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                d_funct7::FCVT_S_D => {
                    // Target = single (fmt=00). Source selected by rs2.
                    c.is_rv32 = true;
                    c.is_f16 = false;
                    match d.rs2.as_u8() {
                        1 => AluOp::FCvtSD, // fcvt.s.d (source double)
                        2 => AluOp::FCvtSH, // fcvt.s.h (source half)
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                h_funct7::FCVT_H_FP => {
                    // Target = half (fmt=10). Source selected by rs2.
                    c.is_rv32 = false;
                    c.is_f16 = true;
                    match d.rs2.as_u8() {
                        0 => AluOp::FCvtHS, // fcvt.h.s (source single)
                        1 => AluOp::FCvtHD, // fcvt.h.d (source double)
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                _ => return Err(Trap::IllegalInstruction(inst)),
            };
        }
        d_opcodes::OP_FMADD | d_opcodes::OP_FMSUB | d_opcodes::OP_FNMADD | d_opcodes::OP_FNMSUB => {
            c.rs1_fp = true;
            c.rs2_fp = true;
            c.rs3_fp = true;
            c.fp_reg_write = true;
            c.b_src = OpBSrc::Reg2;
            let fmt = d.funct7 & 0x3;
            c.is_rv32 = fmt == FP_FMT_SINGLE;
            c.is_f16 = fmt == FP_FMT_HALF;
            let is_double = fmt == FP_FMT_DOUBLE;
            if !c.is_rv32 && !c.is_f16 && !is_double {
                return Err(Trap::IllegalInstruction(inst));
            }
            c.fp_rm = RoundingMode::from_bits(d.funct3 as u8);

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
