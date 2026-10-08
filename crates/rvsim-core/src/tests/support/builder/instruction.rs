use crate::isa::encoding::privileged::{CSRRC, CSRRS, CSRRW, OP_SYSTEM};
use crate::isa::encoding::rv64a::funct5::{AMOADD, LR, SC};
use crate::isa::encoding::rv64a::opcodes::OP_AMO;
use crate::isa::encoding::rv64i::opcodes::*;

/// `fence iorw, iorw`.
pub const FENCE_IORW: u32 = 0x0ff0_000f;
/// `ecall`.
pub const ECALL: u32 = crate::isa::encoding::privileged::ECALL;
/// `mret`.
pub const MRET: u32 = 0x3020_0073;

const FUNCT3_DOUBLE: u32 = 0b011;

pub struct InstructionBuilder {
    opcode: u32,
    rd: u32,
    funct3: u32,
    rs1: u32,
    rs2: u32,
    funct7: u32,
    imm: i32,
}

impl Default for InstructionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl InstructionBuilder {
    pub fn new() -> Self {
        Self { opcode: 0, rd: 0, funct3: 0, rs1: 0, rs2: 0, funct7: 0, imm: 0 }
    }

    pub fn opcode(mut self, op: u32) -> Self {
        self.opcode = op;
        self
    }

    pub fn rd(mut self, rd: u32) -> Self {
        self.rd = rd;
        self
    }

    pub fn rs1(mut self, rs1: u32) -> Self {
        self.rs1 = rs1;
        self
    }

    pub fn rs2(mut self, rs2: u32) -> Self {
        self.rs2 = rs2;
        self
    }

    pub fn funct3(mut self, funct3: u32) -> Self {
        self.funct3 = funct3;
        self
    }

    pub fn funct7(mut self, funct7: u32) -> Self {
        self.funct7 = funct7;
        self
    }

    pub fn add(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.funct7 = 0b0000000;
        self
    }

    pub fn mul(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.funct7 = 0b0000001;
        self
    }

    pub fn div(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b100;
        self.funct7 = 0b0000001;
        self
    }

    pub fn sub(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.funct7 = 0b0100000;
        self
    }

    pub fn addi(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b000;
        self.imm = imm;
        self
    }

    pub fn lbu(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_LOAD;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b100;
        self.imm = imm;
        self
    }

    pub fn lw(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_LOAD;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b010;
        self.imm = imm;
        self
    }

    pub fn sb(mut self, rs1: u32, rs2: u32, imm: i32) -> Self {
        self.opcode = OP_STORE;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.imm = imm;
        self
    }

    pub fn sw(mut self, rs1: u32, rs2: u32, imm: i32) -> Self {
        self.opcode = OP_STORE;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b010;
        self.imm = imm;
        self
    }

    pub fn beq(mut self, rs1: u32, rs2: u32, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.imm = imm;
        self
    }

    pub fn jal(mut self, rd: u32, imm: i32) -> Self {
        self.opcode = OP_JAL;
        self.rd = rd;
        self.imm = imm;
        self
    }

    pub fn jalr(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_JALR;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b000;
        self.imm = imm;
        self
    }

    pub fn lui(mut self, rd: u32, imm: i32) -> Self {
        self.opcode = OP_LUI;
        self.rd = rd;
        self.imm = imm;
        self
    }

    pub fn auipc(mut self, rd: u32, imm: i32) -> Self {
        self.opcode = OP_AUIPC;
        self.rd = rd;
        self.imm = imm;
        self
    }

    // R-type: AND, OR, XOR, SLL, SRL, SRA

    pub fn and(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b111;
        self.funct7 = 0b0000000;
        self
    }

    pub fn or(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b110;
        self.funct7 = 0b0000000;
        self
    }

    pub fn xor(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b100;
        self.funct7 = 0b0000000;
        self
    }

    pub fn sll(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b001;
        self.funct7 = 0b0000000;
        self
    }

    pub fn srl(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b101;
        self.funct7 = 0b0000000;
        self
    }

    pub fn sra(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b101;
        self.funct7 = 0b0100000;
        self
    }

    pub fn slt(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b010;
        self.funct7 = 0b0000000;
        self
    }

    pub fn sltu(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b011;
        self.funct7 = 0b0000000;
        self
    }

    // I-type: ANDI, ORI, XORI, SLTI, SLTIU

    pub fn andi(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b111;
        self.imm = imm;
        self
    }

    pub fn ori(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b110;
        self.imm = imm;
        self
    }

    pub fn xori(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b100;
        self.imm = imm;
        self
    }

    pub fn slti(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b010;
        self.imm = imm;
        self
    }

    pub fn sltiu(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b011;
        self.imm = imm;
        self
    }

    // RV64I: LD, SD, ADDIW, ADDW, SUBW

    pub fn ld(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_LOAD;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b011;
        self.imm = imm;
        self
    }

    pub fn sd(mut self, rs1: u32, rs2: u32, imm: i32) -> Self {
        self.opcode = OP_STORE;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b011;
        self.imm = imm;
        self
    }

    pub fn addiw(mut self, rd: u32, rs1: u32, imm: i32) -> Self {
        self.opcode = OP_IMM_32;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b000;
        self.imm = imm;
        self
    }

    pub fn addw(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG_32;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.funct7 = 0b0000000;
        self
    }

    pub fn subw(mut self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_REG_32;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.funct7 = 0b0100000;
        self
    }

    // Branch variants: BNE, BLT, BGE, BLTU, BGEU

    pub fn bne(mut self, rs1: u32, rs2: u32, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b001;
        self.imm = imm;
        self
    }

    pub fn blt(mut self, rs1: u32, rs2: u32, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b100;
        self.imm = imm;
        self
    }

    pub fn bge(mut self, rs1: u32, rs2: u32, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b101;
        self.imm = imm;
        self
    }

    pub fn bltu(mut self, rs1: u32, rs2: u32, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b110;
        self.imm = imm;
        self
    }

    pub fn bgeu(mut self, rs1: u32, rs2: u32, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b111;
        self.imm = imm;
        self
    }

    /// NOP is ADDI x0, x0, 0
    pub fn nop(self) -> Self {
        self.addi(0, 0, 0)
    }

    /// `csrrw rd, csr, rs1` (`csrw csr, rs1` when `rd` is x0).
    pub fn csrrw(mut self, rd: u32, csr: u32, rs1: u32) -> Self {
        self.opcode = OP_SYSTEM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = CSRRW;
        self.imm = csr as i32;
        self
    }

    /// `csrrc rd, csr, rs1`.
    pub fn csrrc(mut self, rd: u32, csr: u32, rs1: u32) -> Self {
        self.opcode = OP_SYSTEM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = CSRRC;
        self.imm = csr as i32;
        self
    }

    /// `csrrs rd, csr, rs1` (`csrr rd, csr` when `rs1` is x0).
    pub fn csrrs(mut self, rd: u32, csr: u32, rs1: u32) -> Self {
        self.opcode = OP_SYSTEM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = CSRRS;
        self.imm = csr as i32;
        self
    }

    fn amo(mut self, funct5: u32, funct3: u32, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.opcode = OP_AMO;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = funct3;
        self.funct7 = funct5 << 2;
        self
    }

    /// `amoadd.d rd, rs2, (rs1)`.
    pub fn amoadd_d(self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.amo(AMOADD, FUNCT3_DOUBLE, rd, rs1, rs2)
    }

    /// `lr.d rd, (rs1)`.
    pub fn lr_d(self, rd: u32, rs1: u32) -> Self {
        self.amo(LR, FUNCT3_DOUBLE, rd, rs1, 0)
    }

    /// `sc.d rd, rs2, (rs1)`.
    pub fn sc_d(self, rd: u32, rs1: u32, rs2: u32) -> Self {
        self.amo(SC, FUNCT3_DOUBLE, rd, rs1, rs2)
    }

    pub fn build(self) -> u32 {
        let opcode = self.opcode & 0x7F;
        let rd = (self.rd & 0x1F) << 7;
        let funct3 = (self.funct3 & 0x7) << 12;
        let rs1 = (self.rs1 & 0x1F) << 15;
        let rs2 = (self.rs2 & 0x1F) << 20;
        let funct7 = (self.funct7 & 0x7F) << 25;

        match opcode {
            OP_REG | OP_REG_32 | OP_AMO => {
                // R-type: funct7 | rs2 | rs1 | funct3 | rd | opcode
                funct7 | rs2 | rs1 | funct3 | rd | opcode
            }
            OP_IMM | OP_IMM_32 | OP_LOAD | OP_JALR | OP_SYSTEM => {
                // I-type: imm[11:0] | rs1 | funct3 | rd | opcode
                let imm_val = (self.imm as u32) & 0xFFF;
                (imm_val << 20) | rs1 | funct3 | rd | opcode
            }
            OP_STORE => {
                // S-type: imm[11:5] | rs2 | rs1 | funct3 | imm[4:0] | opcode
                let imm_val = self.imm as u32;
                let imm_11_5 = ((imm_val >> 5) & 0x7F) << 25;
                let imm_4_0 = (imm_val & 0x1F) << 7;
                imm_11_5 | rs2 | rs1 | funct3 | imm_4_0 | opcode
            }
            OP_BRANCH => {
                // B-type: imm[12|10:5] | rs2 | rs1 | funct3 | imm[4:1|11] | opcode
                let imm_val = self.imm as u32;
                let bit_12 = ((imm_val >> 12) & 0x1) << 31;
                let bits_10_5 = ((imm_val >> 5) & 0x3F) << 25;
                let bits_4_1 = ((imm_val >> 1) & 0xF) << 8;
                let bit_11 = ((imm_val >> 11) & 0x1) << 7;
                bit_12 | bits_10_5 | rs2 | rs1 | funct3 | bits_4_1 | bit_11 | opcode
            }
            OP_LUI | OP_AUIPC => {
                // U-type: imm is the raw 20-bit immediate (not pre-shifted).
                let imm_val = (self.imm as u32) & 0xFFFFF;
                (imm_val << 12) | rd | opcode
            }
            OP_JAL => {
                // J-type: imm[20|10:1|11|19:12] | rd | opcode
                let imm_val = self.imm as u32;
                let bit_20 = ((imm_val >> 20) & 0x1) << 31;
                let bits_10_1 = ((imm_val >> 1) & 0x3FF) << 21;
                let bit_11 = ((imm_val >> 11) & 0x1) << 20;
                let bits_19_12 = ((imm_val >> 12) & 0xFF) << 12;
                bit_20 | bits_10_1 | bit_11 | bits_19_12 | rd | opcode
            }
            _ => panic!("Unsupported opcode: {:#x}", opcode),
        }
    }
}
