//! Decode Stage: instruction decode and control signal generation.
//!
//! This stage reads from the Fetch2->Decode latch and writes to the
//! Decode->Rename latch. It performs:
//! 1. **Decoding:** Converts raw 32-bit instruction bits into control signals.
//! 2. **Hazard Detection:** Checks for intra-bundle dependencies (superscalar).
//! 3. **Register Read:** Reads source operands (rs1, rs2, rs3) from the register file.
//! 4. **Control Generation:** Generates ALU, Memory, and CSR control signals.

use crate::common::RegIdx;
use crate::common::error::{ExceptionStage, Trap};
use crate::core::Cpu;
use crate::core::pipeline::latches::{IdExEntry, IfIdEntry};
use crate::core::pipeline::signals::{
    AluOp, AtomicOp, ControlFlow, ControlSignals, CsrOp, MemWidth, OpASrc, OpBSrc, SystemOp,
    VecSrcEncoding, VectorOp,
};
use crate::core::units::vpu::types::{Sew, VRegIdx};
use crate::isa::decode::decode as instruction_decode;
use crate::isa::instruction::{Decoded, InstructionBits};
use crate::isa::privileged::opcodes as sys_ops;

use crate::isa::rv64a::{funct3 as a_funct3, funct5 as a_funct5, opcodes as a_opcodes};
use crate::isa::rv64d::{funct7 as d_funct7, opcodes as d_opcodes};
use crate::isa::rv64f::{funct3 as f_funct3, funct7 as f_funct7, opcodes as f_opcodes};
use crate::isa::rv64i::{funct3 as i_funct3, funct7 as i_funct7, opcodes as i_opcodes};
use crate::isa::rv64m::{funct3 as m_funct3, opcodes as m_opcodes};
use crate::isa::rvv::{
    encoding as v_enc, funct3 as v_funct3, funct6 as v_f6, opcodes as v_opcodes,
};

/// ADDI x0, x0, 0 instruction encoding (canonical NOP).
const INSTRUCTION_NOP: u32 = 0x0000_0013;

/// Bit 5 of funct7 field indicating alternate encoding (e.g., SUB vs ADD).
const FUNCT7_ALT_BIT: u32 = 0x20;

/// Floating-point width encoding for 32-bit word operations.
const FP_WIDTH_WORD: u32 = 0x2;

/// Floating-point width encoding for 64-bit double operations.
const FP_WIDTH_DOUBLE: u32 = 0x3;

/// Vector load/store width encoding for EEW=8.
const VEC_WIDTH_8: u32 = 0b000;

/// Vector load/store width encoding for EEW=16.
const VEC_WIDTH_16: u32 = 0b101;

/// Vector load/store width encoding for EEW=32.
const VEC_WIDTH_32: u32 = 0b110;

/// Vector load/store width encoding for EEW=64.
const VEC_WIDTH_64: u32 = 0b111;

/// Unit-stride lumop: normal unit-stride load.
const LUMOP_UNIT: u8 = 0b00000;

/// Unit-stride lumop: whole-register load.
const LUMOP_WHOLE_REG: u8 = 0b01000;

/// Unit-stride lumop: mask load.
const LUMOP_MASK: u8 = 0b01011;

/// Unit-stride lumop: fault-only-first.
const LUMOP_FAULT_FIRST: u8 = 0b10000;

/// Unit-stride sumop: normal unit-stride store.
const SUMOP_UNIT: u8 = 0b00000;

/// Unit-stride sumop: whole-register store.
const SUMOP_WHOLE_REG: u8 = 0b01000;

/// Unit-stride sumop: mask store.
const SUMOP_MASK: u8 = 0b01011;

/// Memory addressing mode (mop): unit-stride.
const MOP_UNIT: u8 = 0b00;

/// Memory addressing mode (mop): indexed unordered.
const MOP_INDEXED_UNORD: u8 = 0b01;

/// Memory addressing mode (mop): strided.
const MOP_STRIDED: u8 = 0b10;

/// Memory addressing mode (mop): indexed ordered.
const MOP_INDEXED_ORD: u8 = 0b11;

/// Floating-point format encoding for single-precision (32-bit).
const FP_FMT_SINGLE: u32 = 0;

/// Floating-point format encoding for double-precision (64-bit).
const FP_FMT_DOUBLE: u32 = 1;

/// Decodes a single instruction into control signals.
fn decode_instruction(inst: u32, pc: u64, d: &Decoded) -> Result<ControlSignals, Trap> {
    let mut c = ControlSignals {
        a_src: OpASrc::Reg1,
        b_src: OpBSrc::Imm,
        alu: AluOp::Add,
        ..Default::default()
    };

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
                i_funct3::SLL => AluOp::Sll,
                i_funct3::SRL_SRA => {
                    if (d.funct7 & FUNCT7_ALT_BIT) != 0 {
                        AluOp::Sra
                    } else {
                        AluOp::Srl
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
            c.mem_read = true;
            c.mem_write = c.atomic_op != AtomicOp::Lr;
            c.reg_write = true;
        }
        f_opcodes::OP_LOAD_FP => {
            match d.funct3 {
                // Scalar FP loads
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
                // Vector loads (EEW encoded in funct3)
                VEC_WIDTH_8 | VEC_WIDTH_16 | VEC_WIDTH_32 | VEC_WIDTH_64 => {
                    decode_vec_load(inst, d.funct3, &mut c)?;
                }
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        f_opcodes::OP_STORE_FP => {
            match d.funct3 {
                // Scalar FP stores
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
                // Vector stores (EEW encoded in funct3)
                VEC_WIDTH_8 | VEC_WIDTH_16 | VEC_WIDTH_32 | VEC_WIDTH_64 => {
                    decode_vec_store(inst, d.funct3, &mut c)?;
                }
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        f_opcodes::OP_FP => {
            let fmt = d.funct7 & 0x3;
            c.is_rv32 = fmt == FP_FMT_SINGLE;
            let is_double = fmt == FP_FMT_DOUBLE;

            if !c.is_rv32 && !is_double {
                return Err(Trap::IllegalInstruction(inst));
            }

            c.rs1_fp = true;
            c.rs2_fp = true;
            c.fp_reg_write = true;
            c.b_src = OpBSrc::Reg2;

            c.alu = match d.funct7 {
                f_funct7::FADD | d_funct7::FADD_D => AluOp::FAdd,
                f_funct7::FSUB | d_funct7::FSUB_D => AluOp::FSub,
                f_funct7::FMUL | d_funct7::FMUL_D => AluOp::FMul,
                f_funct7::FDIV | d_funct7::FDIV_D => AluOp::FDiv,
                f_funct7::FSQRT | d_funct7::FSQRT_D => AluOp::FSqrt,
                f_funct7::FSGNJ | d_funct7::FSGNJ_D => match d.funct3 {
                    f_funct3::FSGNJ => AluOp::FSgnJ,
                    f_funct3::FSGNJN => AluOp::FSgnJN,
                    f_funct3::FSGNJX => AluOp::FSgnJX,
                    _ => return Err(Trap::IllegalInstruction(inst)),
                },
                f_funct7::FMIN_MAX | d_funct7::FMIN_MAX_D => match d.funct3 {
                    f_funct3::FMIN => AluOp::FMin,
                    f_funct3::FMAX => AluOp::FMax,
                    _ => return Err(Trap::IllegalInstruction(inst)),
                },
                f_funct7::FCMP | d_funct7::FCMP_D => {
                    c.fp_reg_write = false;
                    c.reg_write = true;
                    match d.funct3 {
                        f_funct3::FEQ => AluOp::FEq,
                        f_funct3::FLT => AluOp::FLt,
                        f_funct3::FLE => AluOp::FLe,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                f_funct7::FCLASS_MV_X_F | d_funct7::FCLASS_MV_X_D => {
                    c.fp_reg_write = false;
                    c.reg_write = true;
                    c.rs1_fp = true;
                    match d.funct3 {
                        f_funct3::FMV_X_W => AluOp::FMvToX,
                        f_funct3::FCLASS => AluOp::FClass,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    }
                }
                f_funct7::FMV_F_X | d_funct7::FMV_D_X => {
                    c.rs1_fp = false;
                    c.fp_reg_write = true;
                    c.a_src = OpASrc::Reg1;
                    AluOp::FMvToF
                }
                f_funct7::FCVT_W_F | d_funct7::FCVT_W_D => {
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
                f_funct7::FCVT_F_W | d_funct7::FCVT_D_W => {
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
                f_funct7::FCVT_DS => AluOp::FCvtDS,
                d_funct7::FCVT_S_D => AluOp::FCvtSD,
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
            } else {
                match d.raw {
                    sys_ops::EBREAK => return Err(Trap::Breakpoint(pc)),
                    sys_ops::MRET => c.system_op = SystemOp::Mret,
                    sys_ops::SRET => c.system_op = SystemOp::Sret,
                    sys_ops::WFI => c.system_op = SystemOp::Wfi,
                    sys_ops::ECALL => c.system_op = SystemOp::System,
                    _ => {
                        if d.funct3 != 0 {
                            c.system_op = SystemOp::System;
                            c.csr_addr = inst.csr();
                            c.a_src = OpASrc::Reg1;
                            c.b_src = OpBSrc::Zero;
                            c.csr_op = match d.funct3 {
                                sys_ops::CSRRW => CsrOp::Rw,
                                sys_ops::CSRRS => CsrOp::Rs,
                                sys_ops::CSRRC => CsrOp::Rc,
                                sys_ops::CSRRWI => CsrOp::Rwi,
                                sys_ops::CSRRSI => CsrOp::Rsi,
                                sys_ops::CSRRCI => CsrOp::Rci,
                                _ => CsrOp::None,
                            };
                            c.reg_write = !d.rd.is_zero();
                        }
                    }
                }
            }
        }
        i_opcodes::OP_MISC_MEM => match d.funct3 {
            i_funct3::FENCE => c.system_op = SystemOp::Fence,
            i_funct3::FENCE_I => c.system_op = SystemOp::FenceI,
            _ => return Err(Trap::IllegalInstruction(inst)),
        },
        v_opcodes::OP_V => {
            if d.funct3 == v_funct3::OPCFG {
                // vsetvl family: all write scalar rd and are serializing.
                let bit31 = (inst >> 31) & 1;
                let bit30 = (inst >> 30) & 1;

                if bit31 == 0 {
                    // vsetvli: zimm[10:0] from bits 30:20
                    c.vec_op = VectorOp::Vsetvli;
                    c.reg_write = true;
                } else if bit30 == 1 {
                    // vsetivli: zimm[9:0] from bits 29:20, uimm[4:0] from bits 19:15
                    c.vec_op = VectorOp::Vsetivli;
                    c.reg_write = true;
                } else {
                    // vsetvl: vtype from rs2
                    c.vec_op = VectorOp::Vsetvl;
                    c.reg_write = true;
                    c.b_src = OpBSrc::Reg2;
                }
                c.system_op = SystemOp::System;

                // Store decoded vector register indices for downstream
                c.vd = VRegIdx::new(v_enc::vd(inst));
                c.vs1 = VRegIdx::new(v_enc::vs1(inst));
                c.vs2 = VRegIdx::new(v_enc::vs2(inst));
            } else {
                // Vector arithmetic: funct6 is bits 31:26
                let f6 = v_enc::funct6(inst);
                c.vm = v_enc::vm(inst);
                c.vd = VRegIdx::new(v_enc::vd(inst));
                c.vs1 = VRegIdx::new(v_enc::vs1(inst));
                c.vs2 = VRegIdx::new(v_enc::vs2(inst));
                // All vector arithmetic is serializing for Phase 2 (no vector rename yet)
                c.system_op = SystemOp::System;

                match d.funct3 {
                    v_funct3::OPIVV => {
                        c.vec_reg_write = true;
                        c.vec_src_encoding = VecSrcEncoding::VV;
                        c.vec_op = decode_opivv(f6, inst)?;
                    }
                    v_funct3::OPIVX => {
                        c.vec_reg_write = true;
                        c.vec_src_encoding = VecSrcEncoding::VX;
                        c.vec_op = decode_opivx(f6, inst)?;
                    }
                    v_funct3::OPIVI => {
                        c.vec_reg_write = true;
                        c.vec_src_encoding = VecSrcEncoding::VI;
                        c.vec_op = decode_opivi(f6, inst)?;
                    }
                    v_funct3::OPMVV => {
                        c.vec_src_encoding = VecSrcEncoding::VV;
                        let (op, writes_vec) = decode_opmvv(f6, inst)?;
                        c.vec_op = op;
                        c.vec_reg_write = writes_vec;
                        // Some OPMVV ops write scalar rd instead of vec
                        if matches!(op, VectorOp::VMvXS | VectorOp::VCPopM | VectorOp::VFirstM) {
                            c.reg_write = true;
                        }
                    }
                    v_funct3::OPMVX => {
                        c.vec_src_encoding = VecSrcEncoding::VX;
                        let (op, writes_vec) = decode_opmvx(f6, inst)?;
                        c.vec_op = op;
                        c.vec_reg_write = writes_vec;
                    }
                    v_funct3::OPFVV => {
                        c.vec_src_encoding = VecSrcEncoding::VV;
                        let (op, writes_vec) = decode_opfvv(f6, inst)?;
                        c.vec_op = op;
                        c.vec_reg_write = writes_vec;
                        // Scalar-to-FP move (vfmv.f.s) writes FP rd, not vec
                        if matches!(op, VectorOp::VFMvFS) {
                            c.fp_reg_write = true;
                        }
                    }
                    v_funct3::OPFVF => {
                        c.vec_src_encoding = VecSrcEncoding::VF;
                        let (op, writes_vec) = decode_opfvf(f6, inst)?;
                        c.vec_op = op;
                        c.vec_reg_write = writes_vec;
                    }
                    _ => {
                        return Err(Trap::IllegalInstruction(inst));
                    }
                }
            }
        }
        _ => return Err(Trap::IllegalInstruction(inst)),
    }
    Ok(c)
}

// ── Vector arithmetic decode helpers ──────────────────────────────────────────

/// Decode OPIVV funct3 (integer vector-vector) operations.
const fn decode_opivv(f6: u32, inst: u32) -> Result<VectorOp, Trap> {
    Ok(match f6 {
        v_f6::VSUB => VectorOp::VSub,
        v_f6::VMINU => VectorOp::VMinU,
        v_f6::VMIN => VectorOp::VMin,
        v_f6::VMAXU => VectorOp::VMaxU,
        v_f6::VMAX => VectorOp::VMax,
        v_f6::VAND => VectorOp::VAnd,
        v_f6::VOR => VectorOp::VOr,
        v_f6::VXOR => VectorOp::VXor,
        v_f6::VADD => VectorOp::VAdd,
        v_f6::VRGATHER => VectorOp::VRgather,
        v_f6::VADC => VectorOp::VAdc,
        v_f6::VMADC => VectorOp::VMadc,
        v_f6::VSBC => VectorOp::VSbc,
        v_f6::VMSBC => VectorOp::VMsbc,
        v_f6::VMERGE_VMV => VectorOp::VMerge,
        v_f6::VMSEQ => VectorOp::VMSeq,
        v_f6::VMSNE => VectorOp::VMSne,
        v_f6::VMSLTU => VectorOp::VMSltu,
        v_f6::VMSLT => VectorOp::VMSlt,
        v_f6::VMSLEU => VectorOp::VMSleu,
        v_f6::VMSLE => VectorOp::VMSle,
        v_f6::VSADDU => VectorOp::VSAddU,
        v_f6::VSADD => VectorOp::VSAdd,
        v_f6::VSSUBU => VectorOp::VSSubU,
        v_f6::VSSUB => VectorOp::VSSub,
        v_f6::VSLL => VectorOp::VSll,
        v_f6::VSMUL => VectorOp::VSmul,
        v_f6::VSRL => VectorOp::VSrl,
        v_f6::VSRA => VectorOp::VSra,
        v_f6::VSSRL => VectorOp::VSSrl,
        v_f6::VSSRA => VectorOp::VSSra,
        v_f6::VNSRL => VectorOp::VNSrl,
        v_f6::VNSRA => VectorOp::VNSra,
        v_f6::VNCLIPU => VectorOp::VNClipU,
        v_f6::VNCLIP => VectorOp::VNClip,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPIVX funct3 (integer vector-scalar) operations.
const fn decode_opivx(f6: u32, inst: u32) -> Result<VectorOp, Trap> {
    Ok(match f6 {
        v_f6::VSUB => VectorOp::VSub,
        v_f6::VRSUB => VectorOp::VRsub,
        v_f6::VMINU => VectorOp::VMinU,
        v_f6::VMIN => VectorOp::VMin,
        v_f6::VMAXU => VectorOp::VMaxU,
        v_f6::VMAX => VectorOp::VMax,
        v_f6::VAND => VectorOp::VAnd,
        v_f6::VOR => VectorOp::VOr,
        v_f6::VXOR => VectorOp::VXor,
        v_f6::VADD => VectorOp::VAdd,
        v_f6::VSLIDEUP => VectorOp::VSlideUp,
        v_f6::VSLIDEDOWN => VectorOp::VSlideDown,
        v_f6::VADC => VectorOp::VAdc,
        v_f6::VMADC => VectorOp::VMadc,
        v_f6::VSBC => VectorOp::VSbc,
        v_f6::VMSBC => VectorOp::VMsbc,
        v_f6::VMERGE_VMV => VectorOp::VMerge,
        v_f6::VMSEQ => VectorOp::VMSeq,
        v_f6::VMSNE => VectorOp::VMSne,
        v_f6::VMSLTU => VectorOp::VMSltu,
        v_f6::VMSLT => VectorOp::VMSlt,
        v_f6::VMSLEU => VectorOp::VMSleu,
        v_f6::VMSLE => VectorOp::VMSle,
        v_f6::VMSGTU => VectorOp::VMSgtu,
        v_f6::VMSGT => VectorOp::VMSgt,
        v_f6::VSADDU => VectorOp::VSAddU,
        v_f6::VSADD => VectorOp::VSAdd,
        v_f6::VSSUBU => VectorOp::VSSubU,
        v_f6::VSSUB => VectorOp::VSSub,
        v_f6::VSLL => VectorOp::VSll,
        v_f6::VSMUL => VectorOp::VSmul,
        v_f6::VSRL => VectorOp::VSrl,
        v_f6::VSRA => VectorOp::VSra,
        v_f6::VSSRL => VectorOp::VSSrl,
        v_f6::VSSRA => VectorOp::VSSra,
        v_f6::VNSRL => VectorOp::VNSrl,
        v_f6::VNSRA => VectorOp::VNSra,
        v_f6::VNCLIPU => VectorOp::VNClipU,
        v_f6::VNCLIP => VectorOp::VNClip,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPIVI funct3 (integer vector-immediate) operations.
const fn decode_opivi(f6: u32, inst: u32) -> Result<VectorOp, Trap> {
    Ok(match f6 {
        v_f6::VRSUB => VectorOp::VRsub,
        v_f6::VAND => VectorOp::VAnd,
        v_f6::VOR => VectorOp::VOr,
        v_f6::VXOR => VectorOp::VXor,
        v_f6::VADD => VectorOp::VAdd,
        v_f6::VRGATHER => VectorOp::VRgather,
        v_f6::VSLIDEUP => VectorOp::VSlideUp,
        v_f6::VSLIDEDOWN => VectorOp::VSlideDown,
        // Whole-register move: funct6=0b100111 in OPIVI with simm5 encoding nregs
        v_f6::VSMUL => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                0b00000 => VectorOp::VMv1r,
                0b00001 => VectorOp::VMv2r,
                0b00011 => VectorOp::VMv4r,
                0b00111 => VectorOp::VMv8r,
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        v_f6::VADC => VectorOp::VAdc,
        v_f6::VMADC => VectorOp::VMadc,
        v_f6::VMERGE_VMV => VectorOp::VMerge,
        v_f6::VMSEQ => VectorOp::VMSeq,
        v_f6::VMSNE => VectorOp::VMSne,
        v_f6::VMSLEU => VectorOp::VMSleu,
        v_f6::VMSLE => VectorOp::VMSle,
        v_f6::VMSGTU => VectorOp::VMSgtu,
        v_f6::VMSGT => VectorOp::VMSgt,
        v_f6::VSADDU => VectorOp::VSAddU,
        v_f6::VSADD => VectorOp::VSAdd,
        v_f6::VSLL => VectorOp::VSll,
        v_f6::VSRL => VectorOp::VSrl,
        v_f6::VSRA => VectorOp::VSra,
        v_f6::VSSRL => VectorOp::VSSrl,
        v_f6::VSSRA => VectorOp::VSSra,
        v_f6::VNSRL => VectorOp::VNSrl,
        v_f6::VNSRA => VectorOp::VNSra,
        v_f6::VNCLIPU => VectorOp::VNClipU,
        v_f6::VNCLIP => VectorOp::VNClip,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPMVV funct3 (mask/reduction vector-vector) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opmvv(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        // Integer reductions (funct6 0b000000..0b000111)
        v_f6::VREDSUM => (VectorOp::VRedSum, true),
        v_f6::VREDAND => (VectorOp::VRedAnd, true),
        v_f6::VREDOR => (VectorOp::VRedOr, true),
        v_f6::VREDXOR => (VectorOp::VRedXor, true),
        v_f6::VREDMINU => (VectorOp::VRedMinU, true),
        v_f6::VREDMIN => (VectorOp::VRedMin, true),
        v_f6::VREDMAXU => (VectorOp::VRedMaxU, true),
        v_f6::VREDMAX => (VectorOp::VRedMax, true),
        // Averaging add/sub
        v_f6::VAADDU => (VectorOp::VAAddU, true),
        v_f6::VAADD => (VectorOp::VAAdd, true),
        v_f6::VASUBU => (VectorOp::VASubU, true),
        v_f6::VASUB => (VectorOp::VASub, true),
        // Unary ops: vmv.x.s, vcpop.m, vfirst.m (funct6 = 0b010000)
        0b010000 => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                0b00000 => (VectorOp::VMvXS, false),
                0b10000 => (VectorOp::VCPopM, false),
                0b10001 => (VectorOp::VFirstM, false),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        // Mask-producing and misc unary (funct6 = 0b010100)
        0b010100 => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                0b00001 => (VectorOp::VMSbfM, true),
                0b00010 => (VectorOp::VMSofM, true),
                0b00011 => (VectorOp::VMSifM, true),
                0b10000 => (VectorOp::VIotaM, true),
                0b10001 => (VectorOp::VIdV, true),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        // Compress (funct6 = 0b010111, same value as VMERGE_VMV but in OPMVV)
        v_f6::VMERGE_VMV => (VectorOp::VCompress, true),
        // Mask-register logical (funct6 0b011000..0b011111)
        v_f6::VMANDN => (VectorOp::VMAndnMM, true),
        v_f6::VMAND => (VectorOp::VMAndMM, true),
        v_f6::VMOR => (VectorOp::VMOrMM, true),
        v_f6::VMXOR => (VectorOp::VMXorMM, true),
        v_f6::VMORN => (VectorOp::VMOrnMM, true),
        v_f6::VMNAND => (VectorOp::VMNandMM, true),
        v_f6::VMNOR => (VectorOp::VMNorMM, true),
        v_f6::VMXNOR => (VectorOp::VMXnorMM, true),
        // Integer multiply
        v_f6::VMUL => (VectorOp::VMul, true),
        v_f6::VMULH => (VectorOp::VMulh, true),
        v_f6::VMULHU => (VectorOp::VMulhu, true),
        v_f6::VMULHSU => (VectorOp::VMulhsu, true),
        // Multiply-accumulate
        v_f6::VMACC => (VectorOp::VMacc, true),
        v_f6::VNMSAC => (VectorOp::VNMSac, true),
        v_f6::VMADD => (VectorOp::VMadd, true),
        v_f6::VNMSUB => (VectorOp::VNMSub, true),
        // Integer divide
        v_f6::VDIVU => (VectorOp::VDivU, true),
        v_f6::VDIV => (VectorOp::VDiv, true),
        v_f6::VREMU => (VectorOp::VRemU, true),
        v_f6::VREM => (VectorOp::VRem, true),
        // Widening integer add/sub
        // NOTE: VWADDU (0b110000) overlaps VWREDSUMU — decoded as widening add;
        // VWADD (0b110001) overlaps VWREDSUM — decoded as widening add.
        // TODO: Widening reductions share encoding with widening add/sub.
        v_f6::VWADDU => (VectorOp::VWAddU, true),
        v_f6::VWADD => (VectorOp::VWAdd, true),
        v_f6::VWSUBU => (VectorOp::VWSubU, true),
        v_f6::VWSUB => (VectorOp::VWSub, true),
        v_f6::VWADDU_W => (VectorOp::VWAddUW, true),
        v_f6::VWADD_W => (VectorOp::VWAddW, true),
        v_f6::VWSUBU_W => (VectorOp::VWSubUW, true),
        v_f6::VWSUB_W => (VectorOp::VWSubW, true),
        // Widening multiply
        v_f6::VWMULU => (VectorOp::VWMulU, true),
        v_f6::VWMULSU => (VectorOp::VWMulSU, true),
        v_f6::VWMUL => (VectorOp::VWMul, true),
        v_f6::VWMACCU => (VectorOp::VWMaccU, true),
        v_f6::VWMACC => (VectorOp::VWMacc, true),
        v_f6::VWMACCSU => (VectorOp::VWMaccSU, true),
        v_f6::VWMACCUS => (VectorOp::VWMaccUS, true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPMVX funct3 (mask/move vector-scalar) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opmvx(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        // vmv.s.x — move scalar GPR to vector element 0
        0b010000 => (VectorOp::VMvSX, true),
        // vslide1up.vx
        v_f6::VSLIDEUP => (VectorOp::VSlide1Up, true),
        // vslide1down.vx
        v_f6::VSLIDEDOWN => (VectorOp::VSlide1Down, true),
        // Integer multiply
        v_f6::VMUL => (VectorOp::VMul, true),
        v_f6::VMULH => (VectorOp::VMulh, true),
        v_f6::VMULHU => (VectorOp::VMulhu, true),
        v_f6::VMULHSU => (VectorOp::VMulhsu, true),
        // Multiply-accumulate
        v_f6::VMACC => (VectorOp::VMacc, true),
        v_f6::VNMSAC => (VectorOp::VNMSac, true),
        v_f6::VMADD => (VectorOp::VMadd, true),
        v_f6::VNMSUB => (VectorOp::VNMSub, true),
        // Integer divide
        v_f6::VDIVU => (VectorOp::VDivU, true),
        v_f6::VDIV => (VectorOp::VDiv, true),
        v_f6::VREMU => (VectorOp::VRemU, true),
        v_f6::VREM => (VectorOp::VRem, true),
        // Widening integer add/sub
        v_f6::VWADDU => (VectorOp::VWAddU, true),
        v_f6::VWADD => (VectorOp::VWAdd, true),
        v_f6::VWSUBU => (VectorOp::VWSubU, true),
        v_f6::VWSUB => (VectorOp::VWSub, true),
        v_f6::VWADDU_W => (VectorOp::VWAddUW, true),
        v_f6::VWADD_W => (VectorOp::VWAddW, true),
        v_f6::VWSUBU_W => (VectorOp::VWSubUW, true),
        v_f6::VWSUB_W => (VectorOp::VWSubW, true),
        // Widening multiply
        v_f6::VWMULU => (VectorOp::VWMulU, true),
        v_f6::VWMULSU => (VectorOp::VWMulSU, true),
        v_f6::VWMUL => (VectorOp::VWMul, true),
        v_f6::VWMACCU => (VectorOp::VWMaccU, true),
        v_f6::VWMACC => (VectorOp::VWMacc, true),
        v_f6::VWMACCSU => (VectorOp::VWMaccSU, true),
        v_f6::VWMACCUS => (VectorOp::VWMaccUS, true),
        // Averaging add/sub
        v_f6::VAADDU => (VectorOp::VAAddU, true),
        v_f6::VAADD => (VectorOp::VAAdd, true),
        v_f6::VASUBU => (VectorOp::VASubU, true),
        v_f6::VASUB => (VectorOp::VASub, true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

// ── Vector FP decode helpers ──────────────────────────────────────────────────

/// Decode `VFUNARY0` sub-operations (conversion ops) from the vs1 field.
const fn decode_vfunary0(inst: u32) -> Result<VectorOp, Trap> {
    let vs1_field = v_enc::vs1(inst);
    Ok(match vs1_field {
        // Single-width conversions
        0b00000 => VectorOp::VFCvtXuF,
        0b00001 => VectorOp::VFCvtXF,
        0b00010 => VectorOp::VFCvtFXu,
        0b00011 => VectorOp::VFCvtFX,
        0b00110 => VectorOp::VFCvtRtzXuF,
        0b00111 => VectorOp::VFCvtRtzXF,
        // Widening conversions
        0b01000 => VectorOp::VFWCvtXuF,
        0b01001 => VectorOp::VFWCvtXF,
        0b01010 => VectorOp::VFWCvtFXu,
        0b01011 => VectorOp::VFWCvtFX,
        0b01100 => VectorOp::VFWCvtFF,
        0b01110 => VectorOp::VFWCvtRtzXuF,
        0b01111 => VectorOp::VFWCvtRtzXF,
        // Narrowing conversions
        0b10000 => VectorOp::VFNCvtXuF,
        0b10001 => VectorOp::VFNCvtXF,
        0b10010 => VectorOp::VFNCvtFXu,
        0b10011 => VectorOp::VFNCvtFX,
        0b10100 => VectorOp::VFNCvtFF,
        0b10101 => VectorOp::VFNCvtRodFF,
        0b10110 => VectorOp::VFNCvtRtzXuF,
        0b10111 => VectorOp::VFNCvtRtzXF,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode `VFUNARY1` sub-operations (`vfsqrt`, `vfrsqrt7`, `vfrec7`, `vfclass`)
/// from the vs1 field.
const fn decode_vfunary1(inst: u32) -> Result<VectorOp, Trap> {
    let vs1_field = v_enc::vs1(inst);
    Ok(match vs1_field {
        0b00000 => VectorOp::VFSqrt,
        0b00100 => VectorOp::VFRsqrt7,
        0b00101 => VectorOp::VFRec7,
        0b10000 => VectorOp::VFClass,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPFVV funct3 (FP vector-vector) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opfvv(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        // FP arithmetic
        v_f6::VFADD => (VectorOp::VFAdd, true),
        v_f6::VFSUB => (VectorOp::VFSub, true),
        v_f6::VFMIN => (VectorOp::VFMin, true),
        v_f6::VFMAX => (VectorOp::VFMax, true),
        // FP sign injection
        v_f6::VFSGNJ => (VectorOp::VFSgnj, true),
        v_f6::VFSGNJN => (VectorOp::VFSgnjn, true),
        v_f6::VFSGNJX => (VectorOp::VFSgnjx, true),
        // FP comparison (write mask)
        v_f6::VMFEQ => (VectorOp::VMFEq, true),
        v_f6::VMFLE => (VectorOp::VMFLe, true),
        v_f6::VMFLT => (VectorOp::VMFLt, true),
        v_f6::VMFNE => (VectorOp::VMFNe, true),
        // FP divide/multiply
        v_f6::VFDIV => (VectorOp::VFDiv, true),
        v_f6::VFMUL => (VectorOp::VFMul, true),
        // FP fused multiply-add
        v_f6::VFMACC => (VectorOp::VFMacc, true),
        v_f6::VFNMACC => (VectorOp::VFNMacc, true),
        v_f6::VFMSAC => (VectorOp::VFMSac, true),
        v_f6::VFNMSAC => (VectorOp::VFNMSac, true),
        v_f6::VFMADD => (VectorOp::VFMAdd, true),
        v_f6::VFNMADD => (VectorOp::VFNMAdd, true),
        v_f6::VFMSUB => (VectorOp::VFMSub, true),
        v_f6::VFNMSUB => (VectorOp::VFNMSub, true),
        // FP widening arithmetic
        v_f6::VFWADD => (VectorOp::VFWAdd, true),
        v_f6::VFWSUB => (VectorOp::VFWSub, true),
        v_f6::VFWADD_W => (VectorOp::VFWAddW, true),
        v_f6::VFWSUB_W => (VectorOp::VFWSubW, true),
        v_f6::VFWMUL => (VectorOp::VFWMul, true),
        // FP widening FMA
        v_f6::VFWMACC => (VectorOp::VFWMacc, true),
        v_f6::VFWNMACC => (VectorOp::VFWNMacc, true),
        v_f6::VFWMSAC => (VectorOp::VFWMSac, true),
        v_f6::VFWNMSAC => (VectorOp::VFWNMSac, true),
        // FP unary: conversion (VFUNARY0)
        v_f6::VFUNARY0 => match decode_vfunary0(inst) {
            Ok(op) => (op, true),
            Err(e) => return Err(e),
        },
        // FP unary: sqrt/class/rec (VFUNARY1)
        v_f6::VFUNARY1 => match decode_vfunary1(inst) {
            Ok(op) => (op, true),
            Err(e) => return Err(e),
        },
        // FP merge/move (funct6 = 0b010111 in OPFVV = vfmv.f.s)
        v_f6::VMERGE_VMV => (VectorOp::VFMvFS, false),
        // FP reductions
        v_f6::VFREDUSUM => (VectorOp::VFRedUSum, true),
        v_f6::VFREDOSUM => (VectorOp::VFRedOSum, true),
        v_f6::VFREDMIN => (VectorOp::VFRedMin, true),
        v_f6::VFREDMAX => (VectorOp::VFRedMax, true),
        // FP widening reductions
        v_f6::VFWREDUSUM => (VectorOp::VFWRedUSum, true),
        v_f6::VFWREDOSUM => (VectorOp::VFWRedOSum, true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPFVF funct3 (FP vector-scalar) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opfvf(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        // FP arithmetic
        v_f6::VFADD => (VectorOp::VFAdd, true),
        v_f6::VFSUB => (VectorOp::VFSub, true),
        v_f6::VFMIN => (VectorOp::VFMin, true),
        v_f6::VFMAX => (VectorOp::VFMax, true),
        // FP sign injection
        v_f6::VFSGNJ => (VectorOp::VFSgnj, true),
        v_f6::VFSGNJN => (VectorOp::VFSgnjn, true),
        v_f6::VFSGNJX => (VectorOp::VFSgnjx, true),
        // FP comparison (write mask)
        v_f6::VMFEQ => (VectorOp::VMFEq, true),
        v_f6::VMFLE => (VectorOp::VMFLe, true),
        v_f6::VMFLT => (VectorOp::VMFLt, true),
        v_f6::VMFNE => (VectorOp::VMFNe, true),
        v_f6::VMFGT => (VectorOp::VMFGt, true),
        v_f6::VMFGE => (VectorOp::VMFGe, true),
        // FP divide/multiply (OPFVF includes reverse variants)
        v_f6::VFDIV => (VectorOp::VFDiv, true),
        v_f6::VFRDIV => (VectorOp::VFRDiv, true),
        v_f6::VFMUL => (VectorOp::VFMul, true),
        // FP fused multiply-add
        v_f6::VFMACC => (VectorOp::VFMacc, true),
        v_f6::VFNMACC => (VectorOp::VFNMacc, true),
        v_f6::VFMSAC => (VectorOp::VFMSac, true),
        v_f6::VFNMSAC => (VectorOp::VFNMSac, true),
        v_f6::VFMADD => (VectorOp::VFMAdd, true),
        v_f6::VFNMADD => (VectorOp::VFNMAdd, true),
        v_f6::VFMSUB => (VectorOp::VFMSub, true),
        v_f6::VFNMSUB => (VectorOp::VFNMSub, true),
        // FP widening arithmetic
        v_f6::VFWADD => (VectorOp::VFWAdd, true),
        v_f6::VFWSUB => (VectorOp::VFWSub, true),
        v_f6::VFWADD_W => (VectorOp::VFWAddW, true),
        v_f6::VFWSUB_W => (VectorOp::VFWSubW, true),
        v_f6::VFWMUL => (VectorOp::VFWMul, true),
        // FP widening FMA
        v_f6::VFWMACC => (VectorOp::VFWMacc, true),
        v_f6::VFWNMACC => (VectorOp::VFWNMacc, true),
        v_f6::VFWMSAC => (VectorOp::VFWMSac, true),
        v_f6::VFWNMSAC => (VectorOp::VFWNMSac, true),
        // FP slides (OPFVF only)
        v_f6::VFSLIDE1UP => (VectorOp::VFSlide1Up, true),
        v_f6::VFSLIDE1DOWN => (VectorOp::VFSlide1Down, true),
        // FP merge/move (funct6 = 0b010111 in OPFVF)
        // vm=0: vfmerge.vfm, vm=1: vfmv.v.f
        v_f6::VMERGE_VMV => (VectorOp::VFMerge, true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

// ── Vector load/store decode helpers ──────────────────────────────────────────

/// Map funct3 width encoding to `Sew` for vector loads/stores.
const fn funct3_to_eew(funct3: u32) -> Sew {
    match funct3 {
        VEC_WIDTH_8 => Sew::E8,
        VEC_WIDTH_16 => Sew::E16,
        VEC_WIDTH_32 => Sew::E32,
        // VEC_WIDTH_64 and any other value
        _ => Sew::E64,
    }
}

/// Decode a vector load instruction (`OP_LOAD_FP` with vector funct3).
const fn decode_vec_load(inst: u32, funct3: u32, c: &mut ControlSignals) -> Result<(), Trap> {
    let eew = funct3_to_eew(funct3);
    let mop = v_enc::mop(inst);
    let vm = v_enc::vm(inst);
    let nf = v_enc::nf(inst);
    let mew = v_enc::mew(inst);

    // mew must be 0 for RVV 1.0
    if mew {
        return Err(Trap::IllegalInstruction(inst));
    }

    c.vec_eew = eew;
    c.vec_nf = nf;
    c.vm = vm;
    c.vd = VRegIdx::new(v_enc::vd(inst));
    c.vs2 = VRegIdx::new(v_enc::vs2(inst));
    c.vec_reg_write = true;
    c.system_op = SystemOp::System; // serializing

    match mop {
        MOP_UNIT => {
            let lumop = v_enc::lumop(inst);
            c.vec_op = match lumop {
                LUMOP_UNIT => VectorOp::VLoadUnit,
                LUMOP_WHOLE_REG => VectorOp::VLoadWholeReg,
                LUMOP_MASK => VectorOp::VLoadMask,
                LUMOP_FAULT_FIRST => VectorOp::VLoadFF,
                _ => return Err(Trap::IllegalInstruction(inst)),
            };
        }
        MOP_INDEXED_UNORD => {
            c.vec_op = VectorOp::VLoadIndexUnord;
        }
        MOP_STRIDED => {
            c.vec_op = VectorOp::VLoadStride;
        }
        MOP_INDEXED_ORD => {
            c.vec_op = VectorOp::VLoadIndexOrd;
        }
        _ => return Err(Trap::IllegalInstruction(inst)),
    }

    Ok(())
}

/// Decode a vector store instruction (`OP_STORE_FP` with vector funct3).
const fn decode_vec_store(inst: u32, funct3: u32, c: &mut ControlSignals) -> Result<(), Trap> {
    let eew = funct3_to_eew(funct3);
    let mop = v_enc::mop(inst);
    let vm = v_enc::vm(inst);
    let nf = v_enc::nf(inst);
    let mew = v_enc::mew(inst);

    // mew must be 0 for RVV 1.0
    if mew {
        return Err(Trap::IllegalInstruction(inst));
    }

    c.vec_eew = eew;
    c.vec_nf = nf;
    c.vm = vm;
    c.vd = VRegIdx::new(v_enc::vd(inst)); // vd is vs3 (store data) for stores
    c.vs2 = VRegIdx::new(v_enc::vs2(inst));
    c.vec_reg_write = false; // stores don't write vector registers
    c.system_op = SystemOp::System; // serializing

    match mop {
        MOP_UNIT => {
            let sumop = v_enc::sumop(inst);
            c.vec_op = match sumop {
                SUMOP_UNIT => VectorOp::VStoreUnit,
                SUMOP_WHOLE_REG => VectorOp::VStoreWholeReg,
                SUMOP_MASK => VectorOp::VStoreMask,
                _ => return Err(Trap::IllegalInstruction(inst)),
            };
        }
        MOP_INDEXED_UNORD => {
            c.vec_op = VectorOp::VStoreIndexUnord;
        }
        MOP_STRIDED => {
            c.vec_op = VectorOp::VStoreStride;
        }
        MOP_INDEXED_ORD => {
            c.vec_op = VectorOp::VStoreIndexOrd;
        }
        _ => return Err(Trap::IllegalInstruction(inst)),
    }

    Ok(())
}

/// Executes the decode stage.
///
/// Consumes Fetch2->Decode entries (`IfIdEntry`) and produces
/// Decode->Rename entries (`IdExEntry`).
pub fn decode_stage(cpu: &mut Cpu, input: &mut Vec<IfIdEntry>, output: &mut Vec<IdExEntry>) {
    let mut consumed_count = 0;
    let mut bundle_writes: Vec<(RegIdx, bool)> = Vec::with_capacity(cpu.pipeline_width);
    let mut broke_on_trap = false;

    for if_entry in input.iter() {
        if let Some(trap) = &if_entry.trap {
            output.push(IdExEntry {
                pc: if_entry.pc,
                inst: if_entry.inst,
                inst_size: if_entry.inst_size,
                trap: Some(trap.clone()),
                exception_stage: if_entry.exception_stage,
                ..Default::default()
            });
            consumed_count += 1;
            broke_on_trap = true;
            break;
        }

        let inst = if_entry.inst;

        if inst == INSTRUCTION_NOP {
            consumed_count += 1;
            continue;
        }

        let d = instruction_decode(inst);

        let (mut ctrl, trap, ex_stage) = match decode_instruction(inst, if_entry.pc, &d) {
            Ok(c) => (c, None, None),
            Err(t) => (ControlSignals::default(), Some(t), Some(ExceptionStage::Decode)),
        };

        // Set vec_lmul_regs from the current vtype CSR for vector data ops.
        // vsetvl family instructions are config ops and don't use LMUL groups.
        // Since vsetvl is serializing, vtype is always up-to-date at decode.
        if ctrl.vec_op != VectorOp::None
            && !matches!(ctrl.vec_op, VectorOp::Vsetvli | VectorOp::Vsetivli | VectorOp::Vsetvl)
        {
            let vtype = crate::core::units::vpu::types::parse_vtype(cpu.csrs.vtype);
            if !vtype.vill {
                ctrl.vec_lmul_regs = vtype.vlmul.group_regs().regs();
            }
        }

        // Check for intra-bundle hazards (superscalar, in-order only).
        // With register renaming (O3 backend), rename resolves all RAW hazards
        // by mapping source operands to physical registers before updating the
        // rename map for the destination. Splitting the bundle here would
        // create unnecessary 1-cycle bubbles.
        let rs3_idx = inst.rs3();
        if !cpu.has_register_renaming {
            let hazard = ((!d.rs1.is_zero() || ctrl.rs1_fp)
                && bundle_writes.contains(&(d.rs1, ctrl.rs1_fp)))
                || ((!d.rs2.is_zero() || ctrl.rs2_fp)
                    && bundle_writes.contains(&(d.rs2, ctrl.rs2_fp)))
                || (ctrl.rs3_fp && bundle_writes.contains(&(rs3_idx, true)));

            if hazard {
                break;
            }
        }

        if ctrl.reg_write && !d.rd.is_zero() {
            bundle_writes.push((d.rd, false));
        }
        if ctrl.fp_reg_write {
            bundle_writes.push((d.rd, true));
        }

        let rv1 = if ctrl.rs1_fp { cpu.regs.read_f(d.rs1) } else { cpu.regs.read(d.rs1) };
        let rv2 = if ctrl.rs2_fp { cpu.regs.read_f(d.rs2) } else { cpu.regs.read(d.rs2) };
        let rv3 = if ctrl.rs3_fp { cpu.regs.read_f(rs3_idx) } else { 0 };

        let has_trap = trap.is_some();

        output.push(IdExEntry {
            pc: if_entry.pc,
            inst,
            inst_size: if_entry.inst_size,
            rs1: d.rs1,
            rs2: d.rs2,
            rs3: rs3_idx,
            rd: d.rd,
            imm: d.imm,
            rv1,
            rv2,
            rv3,
            ctrl,
            trap,
            exception_stage: ex_stage,
            pred_taken: if_entry.pred_taken,
            pred_target: if_entry.pred_target,
            ghr_snapshot: if_entry.ghr_snapshot,
            ras_snapshot: if_entry.ras_snapshot,
        });

        consumed_count += 1;

        if has_trap {
            broke_on_trap = true;
            break;
        }
    }

    // Remove consumed entries from input, keeping unconsumed ones
    if broke_on_trap || consumed_count >= input.len() {
        let _ = input.drain(..consumed_count);
    } else {
        // Intra-bundle hazard stopped us early — drain consumed entries
        let _ = input.drain(..consumed_count);
    }
}
