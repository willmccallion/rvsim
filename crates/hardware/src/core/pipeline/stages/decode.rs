//! Instruction Decode (ID) Stage.
//!
//! This module implements the second stage of the pipeline. It performs the following:
//! 1. **Decoding:** Converts raw 32-bit instruction bits into control signals using the ISA decoder.
//! 2. **Hazard Detection:** Checks for intra-bundle dependencies (in superscalar configurations).
//! 3. **Register Read:** Reads source operands (rs1, rs2, rs3) from the Register File.
//! 4. **Control Generation:** Generates ALU, Memory, and CSR control signals for the Execute stage.

use crate::common::error::Trap;
use crate::core::Cpu;
use crate::core::pipeline::latches::IdExEntry;
use crate::core::pipeline::signals::{
    AluOp, AtomicOp, ControlSignals, CsrOp, MemWidth, OpASrc, OpBSrc,
};
use crate::isa::decode::decode as instruction_decode;
use crate::isa::instruction::{Decoded, InstructionBits};
use crate::isa::privileged::opcodes as sys_ops;

use crate::isa::rv64a::{funct3 as a_funct3, funct5 as a_funct5, opcodes as a_opcodes};
use crate::isa::rv64d::{funct7 as d_funct7, opcodes as d_opcodes};
use crate::isa::rv64f::{funct3 as f_funct3, funct7 as f_funct7, opcodes as f_opcodes};
use crate::isa::rv64i::{funct3 as i_funct3, funct7 as i_funct7, opcodes as i_opcodes};
use crate::isa::rv64m::{funct3 as m_funct3, opcodes as m_opcodes};

/// ADDI x0, x0, 0 instruction encoding (canonical NOP).
///
/// This instruction performs no operation and is used to flush pipeline stages.
const INSTRUCTION_NOP: u32 = 0x0000_0013;

/// Zero instruction encoding (invalid instruction used as NOP).
///
/// Treated as a no-op when decoded; used to pad or flush the pipeline.
const INSTRUCTION_ZERO: u32 = 0;

/// Bit 5 of funct7 field indicating alternate encoding (e.g., SUB vs ADD).
///
/// When set, selects the alternate R-type operation (e.g., SRA instead of SRL).
const FUNCT7_ALT_BIT: u32 = 0x20;

/// Floating-point width encoding for 32-bit word operations.
///
/// Used in FP load/store `funct3` to select single-precision width.
const FP_WIDTH_WORD: u32 = 0x2;

/// Floating-point width encoding for 64-bit double operations.
///
/// Used in FP load/store `funct3` to select double-precision width.
const FP_WIDTH_DOUBLE: u32 = 0x3;

/// Floating-point format encoding for single-precision (32-bit).
///
/// Used in FP op `funct7` format field to select 32-bit operands.
const FP_FMT_SINGLE: u32 = 0;

/// Floating-point format encoding for double-precision (64-bit).
///
/// Used in FP op `funct7` format field to select 64-bit operands.
const FP_FMT_DOUBLE: u32 = 1;

/// Executes the instruction decode stage.
///
/// This function processes instructions from the IF/ID latch. It decodes the raw instruction
/// bits into control signals, reads source operands from the register file (handling
/// intra-bundle hazards if superscalar), and pushes the result to the ID/EX latch.
///
/// # Arguments
///
/// * `cpu` - Mutable reference to the CPU state.
pub fn decode_stage(cpu: &mut Cpu) {
    let mut if_entries = std::mem::take(&mut cpu.if_id.entries);

    let mut id_ex_entries = std::mem::take(&mut cpu.id_ex_shadow);
    id_ex_entries.clear();

    let mut consumed_count = 0;
    let mut bundle_writes: Vec<(usize, bool)> = Vec::with_capacity(cpu.pipeline_width);

    for if_entry in &if_entries {
        if let Some(trap) = &if_entry.trap {
            id_ex_entries.push(IdExEntry {
                pc: if_entry.pc,
                inst: if_entry.inst,
                inst_size: if_entry.inst_size,
                trap: Some(trap.clone()),
                ..Default::default()
            });
            consumed_count += 1;
            continue;
        }

        let inst = if_entry.inst;

        if inst == INSTRUCTION_NOP || inst == INSTRUCTION_ZERO {
            consumed_count += 1;
            continue;
        }

        let d = instruction_decode(inst);

        let decode_result = |d: &Decoded| -> Result<ControlSignals, Trap> {
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
                    c.jump = true;
                }
                i_opcodes::OP_JALR => {
                    c.reg_write = true;
                    c.jump = true;
                    c.alu = AluOp::Add;
                }
                i_opcodes::OP_BRANCH => {
                    c.branch = true;
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
                    c.fp_reg_write = true;
                    c.mem_read = true;
                    c.alu = AluOp::Add;
                    c.width = match d.funct3 {
                        FP_WIDTH_WORD => MemWidth::Word,
                        FP_WIDTH_DOUBLE => MemWidth::Double,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    };
                }
                f_opcodes::OP_STORE_FP => {
                    c.mem_write = true;
                    c.rs1_fp = false;
                    c.rs2_fp = true;
                    c.b_src = OpBSrc::Imm;
                    c.alu = AluOp::Add;
                    c.width = match d.funct3 {
                        FP_WIDTH_WORD => MemWidth::Word,
                        FP_WIDTH_DOUBLE => MemWidth::Double,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    };
                }
                f_opcodes::OP_FP => {
                    let fmt = (d.funct7 >> 0) & 0x3;
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
                            if d.rs2 == 0 || d.rs2 == 1 {
                                AluOp::FCvtWS
                            } else {
                                AluOp::FCvtLS
                            }
                        }
                        f_funct7::FCVT_F_W | d_funct7::FCVT_D_W => {
                            c.rs1_fp = false;
                            c.fp_reg_write = true;
                            c.a_src = OpASrc::Reg1;
                            if d.rs2 == 0 || d.rs2 == 1 {
                                AluOp::FCvtSW
                            } else {
                                AluOp::FCvtSL
                            }
                        }
                        f_funct7::FCVT_DS => AluOp::FCvtDS,
                        d_funct7::FCVT_S_D => AluOp::FCvtSD,
                        _ => return Err(Trap::IllegalInstruction(inst)),
                    };
                }
                d_opcodes::OP_FMADD
                | d_opcodes::OP_FMSUB
                | d_opcodes::OP_FNMADD
                | d_opcodes::OP_FNMSUB => {
                    c.rs1_fp = true;
                    c.rs2_fp = true;
                    c.rs3_fp = true;
                    c.fp_reg_write = true;
                    c.b_src = OpBSrc::Reg2;
                    let fmt = (d.funct7 >> 0) & 0x3;
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
                    c.is_system = true;
                    match d.raw {
                        sys_ops::ECALL => {}
                        sys_ops::EBREAK => return Err(Trap::Breakpoint(if_entry.pc)),
                        sys_ops::MRET => c.is_mret = true,
                        sys_ops::SRET => c.is_sret = true,
                        sys_ops::WFI => {}
                        sys_ops::SFENCE_VMA => {}
                        _ => {
                            if d.funct3 != 0 {
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
                                match c.csr_op {
                                    CsrOp::Rwi | CsrOp::Rsi | CsrOp::Rci => {
                                        c.reg_write = d.rd != 0;
                                    }
                                    _ => {
                                        c.reg_write = d.rd != 0;
                                    }
                                }
                            }
                        }
                    }
                }
                i_opcodes::OP_MISC_MEM => match d.funct3 {
                    i_funct3::FENCE => {}
                    i_funct3::FENCE_I => c.is_fence_i = true,
                    _ => return Err(Trap::IllegalInstruction(inst)),
                },
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
            Ok(c)
        };

        let (ctrl, trap) = match decode_result(&d) {
            Ok(c) => (c, None),
            Err(t) => (ControlSignals::default(), Some(t)),
        };

        let mut hazard = false;
        if d.rs1 != 0 || ctrl.rs1_fp {
            if bundle_writes.contains(&(d.rs1, ctrl.rs1_fp)) {
                hazard = true;
            }
        }
        if d.rs2 != 0 || ctrl.rs2_fp {
            if bundle_writes.contains(&(d.rs2, ctrl.rs2_fp)) {
                hazard = true;
            }
        }
        let rs3_idx = inst.rs3();
        if ctrl.rs3_fp {
            if bundle_writes.contains(&(rs3_idx, true)) {
                hazard = true;
            }
        }

        if hazard {
            break;
        }

        if ctrl.reg_write && d.rd != 0 {
            bundle_writes.push((d.rd, false));
        }
        if ctrl.fp_reg_write {
            bundle_writes.push((d.rd, true));
        }

        let rv1 = if ctrl.rs1_fp {
            cpu.regs.read_f(d.rs1)
        } else {
            cpu.regs.read(d.rs1)
        };
        let rv2 = if ctrl.rs2_fp {
            cpu.regs.read_f(d.rs2)
        } else {
            cpu.regs.read(d.rs2)
        };
        let rv3 = if ctrl.rs3_fp {
            cpu.regs.read_f(rs3_idx)
        } else {
            0
        };

        id_ex_entries.push(IdExEntry {
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
            pred_taken: if_entry.pred_taken,
            pred_target: if_entry.pred_target,
        });

        consumed_count += 1;
    }

    if consumed_count < if_entries.len() {
        let remaining = if_entries.split_off(consumed_count);
        cpu.if_id.entries = remaining;
    }

    cpu.id_ex.entries = id_ex_entries;
    cpu.if_id_shadow = if_entries;
}
