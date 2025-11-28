use crate::cpu::Cpu;
use crate::cpu::control::{AluOp, ControlSignals, CsrOp, MemWidth, OpASrc, OpBSrc};
use crate::cpu::pipeline::{IDEx, bubble_idex};
use crate::isa::{decoder, funct3, funct7, opcodes};

pub fn decode_stage(cpu: &mut Cpu) -> Result<(), String> {
    let inst = cpu.if_id.inst;

    if inst == 0x0000_0013 || inst == 0x0000_0000 {
        cpu.id_ex = bubble_idex();
        return Ok(());
    }

    let pc = cpu.if_id.pc;
    if cpu.trace {
        eprintln!("ID  pc={:#x} inst={:#010x}", pc, inst);
    }

    let d = decoder::decode(inst);

    let decode_logic = |d: &crate::isa::instruction::Decoded| -> Result<ControlSignals, String> {
        let mut c = ControlSignals::default();
        c.a_src = OpASrc::Reg1;
        c.b_src = OpBSrc::Imm;
        c.alu = AluOp::Add;

        match d.opcode {
            opcodes::OP_LUI => {
                c.reg_write = true;
                c.a_src = OpASrc::Zero;
            }
            opcodes::OP_AUIPC => {
                c.reg_write = true;
                c.a_src = OpASrc::Pc;
            }
            opcodes::OP_JAL => {
                c.reg_write = true;
                c.jump = true;
            }
            opcodes::OP_JALR => {
                c.reg_write = true;
                c.jump = true;
                c.alu = AluOp::Add;
            }
            opcodes::OP_BRANCH => {
                c.branch = true;
                c.b_src = OpBSrc::Reg2;
            }
            opcodes::OP_LOAD => {
                c.reg_write = true;
                c.mem_read = true;
                c.alu = AluOp::Add;
                let (w, s) = match d.funct3 {
                    funct3::LB => (MemWidth::Byte, true),
                    funct3::LH => (MemWidth::Half, true),
                    funct3::LW => (MemWidth::Word, true),
                    funct3::LD => (MemWidth::Double, true),
                    funct3::LBU => (MemWidth::Byte, false),
                    funct3::LHU => (MemWidth::Half, false),
                    funct3::LWU => (MemWidth::Word, false),
                    _ => return Err(format!("illegal load funct3 {}", d.funct3)),
                };
                c.width = w;
                c.signed_load = s;
            }
            opcodes::OP_STORE => {
                c.mem_write = true;
                c.b_src = OpBSrc::Imm;
                c.alu = AluOp::Add;
                c.width = match d.funct3 {
                    funct3::SB => MemWidth::Byte,
                    funct3::SH => MemWidth::Half,
                    funct3::SW => MemWidth::Word,
                    funct3::SD => MemWidth::Double,
                    _ => return Err(format!("illegal store funct3 {}", d.funct3)),
                };
            }
            opcodes::OP_IMM | opcodes::OP_IMM_32 => {
                c.reg_write = true;
                c.is_rv32 = d.opcode == opcodes::OP_IMM_32;
                c.alu = match d.funct3 {
                    funct3::ADD_SUB => AluOp::Add,
                    funct3::SLT => AluOp::Slt,
                    funct3::SLTU => AluOp::Sltu,
                    funct3::XOR => AluOp::Xor,
                    funct3::OR => AluOp::Or,
                    funct3::AND => AluOp::And,
                    funct3::SLL => AluOp::Sll,
                    funct3::SRL_SRA => {
                        if d.funct7 & 0x20 != 0 {
                            AluOp::Sra
                        } else {
                            AluOp::Srl
                        }
                    }
                    _ => return Err("illegal OP_IMM".into()),
                };
            }
            opcodes::OP_REG | opcodes::OP_REG_32 => {
                c.reg_write = true;
                c.is_rv32 = d.opcode == opcodes::OP_REG_32;
                c.b_src = OpBSrc::Reg2;
                c.alu = match (d.funct3, d.funct7) {
                    (funct3::ADD_SUB, funct7::DEFAULT) => AluOp::Add,
                    (funct3::ADD_SUB, funct7::SUB) => AluOp::Sub,
                    (funct3::SLL, funct7::DEFAULT) => AluOp::Sll,
                    (funct3::SLT, funct7::DEFAULT) => AluOp::Slt,
                    (funct3::SLTU, funct7::DEFAULT) => AluOp::Sltu,
                    (funct3::XOR, funct7::DEFAULT) => AluOp::Xor,
                    (funct3::SRL_SRA, funct7::DEFAULT) => AluOp::Srl,
                    (funct3::SRL_SRA, funct7::SRA) => AluOp::Sra,
                    (funct3::OR, funct7::DEFAULT) => AluOp::Or,
                    (funct3::AND, funct7::DEFAULT) => AluOp::And,
                    (funct3::ADD_SUB, funct7::M_EXTENSION) => AluOp::Mul,
                    (funct3::SLL, funct7::M_EXTENSION) => AluOp::Mulh,
                    (funct3::SLT, funct7::M_EXTENSION) => AluOp::Mulhsu,
                    (funct3::SLTU, funct7::M_EXTENSION) => AluOp::Mulhu,
                    (funct3::XOR, funct7::M_EXTENSION) => AluOp::Div,
                    (funct3::SRL_SRA, funct7::M_EXTENSION) => AluOp::Divu,
                    (funct3::OR, funct7::M_EXTENSION) => AluOp::Rem,
                    (funct3::AND, funct7::M_EXTENSION) => AluOp::Remu,
                    _ => return Err("illegal OP_REG".into()),
                };
            }
            opcodes::OP_SYSTEM => {
                c.is_system = true;
                match d.raw {
                    0x0000_0073 => { /* ECALL */ }
                    0x0010_0073 => { /* EBREAK */ }
                    0x3020_0073 => {
                        c.is_mret = true;
                    }
                    0x1020_0073 => {
                        c.is_sret = true;
                    }
                    _ => {
                        c.csr_addr = d.raw >> 20;
                        c.a_src = OpASrc::Reg1;
                        c.b_src = OpBSrc::Zero;
                        c.csr_op = match d.funct3 {
                            0b001 => CsrOp::RW,
                            0b010 => CsrOp::RS,
                            0b011 => CsrOp::RC,
                            0b101 => CsrOp::RWI,
                            0b110 => CsrOp::RSI,
                            0b111 => CsrOp::RCI,
                            _ => CsrOp::None,
                        };
                        if c.csr_op == CsrOp::None {
                            return Err(format!("illegal SYSTEM funct3 {:#b}", d.funct3));
                        }
                        c.reg_write = d.rd != 0;
                    }
                }
            }
            _ => return Err(format!("illegal opcode {:#x}", d.opcode)),
        }
        Ok(c)
    };

    let (ctrl, trap) = match decode_logic(&d) {
        Ok(c) => (c, None),
        Err(msg) => (ControlSignals::default(), Some(msg)),
    };

    cpu.id_ex = IDEx {
        pc,
        inst,
        rs1: d.rs1,
        rs2: d.rs2,
        rd: d.rd,
        imm: d.imm,
        rv1: cpu.regs.read(d.rs1),
        rv2: cpu.regs.read(d.rs2),
        ctrl,
        trap,
    };

    Ok(())
}
