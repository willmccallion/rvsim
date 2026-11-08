use crate::isa::csr::{MCAUSE, MTVAL};
use crate::isa::encode::{BType, IType, JType, RType, SType, UType};
use crate::isa::encoding::privileged::{CSRRC, CSRRS, CSRRW, OP_SYSTEM};
use crate::isa::encoding::rv64a::funct5::{AMOADD, LR, SC};
use crate::isa::encoding::rv64a::opcodes::OP_AMO;
use crate::isa::encoding::rv64i::opcodes::*;
use crate::isa::op::pseudo::PseudoOp;
use crate::isa::reg::RegIdx;
use crate::tests::support::regs::{A0, A1, A2, A3, ZERO};

/// `jal x0, 0`: a loop of one instruction, where a test program ends.
pub const JAL_SELF: u32 = 0x0000_006F;
/// `fence iorw, iorw`.
pub const FENCE_IORW: u32 = 0x0ff0_000f;
/// `fence.i`.
pub const FENCE_I: u32 = 0x0000_100f;
/// `ecall`.
pub use crate::isa::encoding::privileged::ECALL;
/// gem5's `m5_fail`: ends the run with `a1` as the exit code.
pub const M5_FAIL: u32 = PseudoOp::encode(0x22);

/// Ends the run with `a0` as the exit code, as a program's exit does.
#[must_use]
pub fn exit_with_a0() -> [u32; 2] {
    [InstructionBuilder::new().addi(A1, A0, 0).build(), M5_FAIL]
}
/// A machine-mode trap handler that records `mcause` in `a2` and `mtval`
/// in `a3`, then spins.
#[must_use]
pub fn trap_recorder() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().csrrs(A2, MCAUSE.as_u32(), ZERO).build(),
        i().csrrs(A3, MTVAL.as_u32(), ZERO).build(),
        i().jal(ZERO, 0).build(),
    ]
}
/// `mret`.
pub const MRET: u32 = 0x3020_0073;

const FUNCT3_DOUBLE: u32 = 0b011;

pub struct InstructionBuilder {
    opcode: u32,
    rd: RegIdx,
    funct3: u32,
    rs1: RegIdx,
    rs2: RegIdx,
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
        Self { opcode: 0, rd: ZERO, funct3: 0, rs1: ZERO, rs2: ZERO, funct7: 0, imm: 0 }
    }

    pub fn opcode(mut self, op: u32) -> Self {
        self.opcode = op;
        self
    }

    pub fn rd(mut self, rd: RegIdx) -> Self {
        self.rd = rd;
        self
    }

    pub fn rs1(mut self, rs1: RegIdx) -> Self {
        self.rs1 = rs1;
        self
    }

    pub fn rs2(mut self, rs2: RegIdx) -> Self {
        self.rs2 = rs2;
        self
    }

    pub fn funct7(mut self, funct7: u32) -> Self {
        self.funct7 = funct7;
        self
    }

    pub fn add(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.funct7 = 0b0000000;
        self
    }

    pub fn mul(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.funct7 = 0b0000001;
        self
    }

    pub fn div(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b100;
        self.funct7 = 0b0000001;
        self
    }

    pub fn sub(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.funct7 = 0b0100000;
        self
    }

    pub fn addi(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b000;
        self.imm = imm;
        self
    }

    pub fn lbu(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_LOAD;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b100;
        self.imm = imm;
        self
    }

    pub fn lw(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_LOAD;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b010;
        self.imm = imm;
        self
    }

    pub fn sb(mut self, rs1: RegIdx, rs2: RegIdx, imm: i32) -> Self {
        self.opcode = OP_STORE;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.imm = imm;
        self
    }

    pub fn sw(mut self, rs1: RegIdx, rs2: RegIdx, imm: i32) -> Self {
        self.opcode = OP_STORE;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b010;
        self.imm = imm;
        self
    }

    pub fn beq(mut self, rs1: RegIdx, rs2: RegIdx, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.imm = imm;
        self
    }

    pub fn jal(mut self, rd: RegIdx, imm: i32) -> Self {
        self.opcode = OP_JAL;
        self.rd = rd;
        self.imm = imm;
        self
    }

    pub fn jalr(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_JALR;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b000;
        self.imm = imm;
        self
    }

    pub fn lui(mut self, rd: RegIdx, imm: i32) -> Self {
        self.opcode = OP_LUI;
        self.rd = rd;
        self.imm = imm;
        self
    }

    pub fn auipc(mut self, rd: RegIdx, imm: i32) -> Self {
        self.opcode = OP_AUIPC;
        self.rd = rd;
        self.imm = imm;
        self
    }

    // R-type: AND, OR, XOR, SLL, SRL, SRA

    pub fn and(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b111;
        self.funct7 = 0b0000000;
        self
    }

    pub fn or(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b110;
        self.funct7 = 0b0000000;
        self
    }

    pub fn xor(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b100;
        self.funct7 = 0b0000000;
        self
    }

    pub fn sll(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b001;
        self.funct7 = 0b0000000;
        self
    }

    pub fn srl(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b101;
        self.funct7 = 0b0000000;
        self
    }

    pub fn sra(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b101;
        self.funct7 = 0b0100000;
        self
    }

    pub fn slt(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b010;
        self.funct7 = 0b0000000;
        self
    }

    pub fn sltu(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b011;
        self.funct7 = 0b0000000;
        self
    }

    // I-type: ANDI, ORI, XORI, SLTI, SLTIU

    pub fn andi(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b111;
        self.imm = imm;
        self
    }

    pub fn ori(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b110;
        self.imm = imm;
        self
    }

    pub fn xori(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b100;
        self.imm = imm;
        self
    }

    pub fn slti(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b010;
        self.imm = imm;
        self
    }

    pub fn sltiu(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_IMM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b011;
        self.imm = imm;
        self
    }

    // RV64I: LD, SD, ADDIW, ADDW, SUBW

    pub fn ld(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_LOAD;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b011;
        self.imm = imm;
        self
    }

    pub fn sd(mut self, rs1: RegIdx, rs2: RegIdx, imm: i32) -> Self {
        self.opcode = OP_STORE;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b011;
        self.imm = imm;
        self
    }

    pub fn addiw(mut self, rd: RegIdx, rs1: RegIdx, imm: i32) -> Self {
        self.opcode = OP_IMM_32;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = 0b000;
        self.imm = imm;
        self
    }

    pub fn addw(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG_32;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.funct7 = 0b0000000;
        self
    }

    pub fn subw(mut self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_REG_32;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b000;
        self.funct7 = 0b0100000;
        self
    }

    // Branch variants: BNE, BLT, BGE, BLTU, BGEU

    pub fn bne(mut self, rs1: RegIdx, rs2: RegIdx, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b001;
        self.imm = imm;
        self
    }

    pub fn blt(mut self, rs1: RegIdx, rs2: RegIdx, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b100;
        self.imm = imm;
        self
    }

    pub fn bge(mut self, rs1: RegIdx, rs2: RegIdx, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b101;
        self.imm = imm;
        self
    }

    pub fn bltu(mut self, rs1: RegIdx, rs2: RegIdx, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b110;
        self.imm = imm;
        self
    }

    pub fn bgeu(mut self, rs1: RegIdx, rs2: RegIdx, imm: i32) -> Self {
        self.opcode = OP_BRANCH;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = 0b111;
        self.imm = imm;
        self
    }

    /// NOP is ADDI x0, x0, 0
    pub fn nop(self) -> Self {
        self.addi(ZERO, ZERO, 0)
    }

    /// `csrrw rd, csr, rs1` (`csrw csr, rs1` when `rd` is x0).
    pub fn csrrw(mut self, rd: RegIdx, csr: u32, rs1: RegIdx) -> Self {
        self.opcode = OP_SYSTEM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = CSRRW;
        self.imm = csr.cast_signed();
        self
    }

    /// `csrrc rd, csr, rs1`.
    pub fn csrrc(mut self, rd: RegIdx, csr: u32, rs1: RegIdx) -> Self {
        self.opcode = OP_SYSTEM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = CSRRC;
        self.imm = csr.cast_signed();
        self
    }

    /// `csrrs rd, csr, rs1` (`csrr rd, csr` when `rs1` is x0).
    pub fn csrrs(mut self, rd: RegIdx, csr: u32, rs1: RegIdx) -> Self {
        self.opcode = OP_SYSTEM;
        self.rd = rd;
        self.rs1 = rs1;
        self.funct3 = CSRRS;
        self.imm = csr.cast_signed();
        self
    }

    fn amo(mut self, funct5: u32, funct3: u32, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.opcode = OP_AMO;
        self.rd = rd;
        self.rs1 = rs1;
        self.rs2 = rs2;
        self.funct3 = funct3;
        self.funct7 = funct5 << 2;
        self
    }

    /// `amoadd.d rd, rs2, (rs1)`.
    pub fn amoadd_d(self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.amo(AMOADD, FUNCT3_DOUBLE, rd, rs1, rs2)
    }

    /// `lr.d rd, (rs1)`.
    pub fn lr_d(self, rd: RegIdx, rs1: RegIdx) -> Self {
        self.amo(LR, FUNCT3_DOUBLE, rd, rs1, ZERO)
    }

    /// `sc.d rd, rs2, (rs1)`.
    pub fn sc_d(self, rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> Self {
        self.amo(SC, FUNCT3_DOUBLE, rd, rs1, rs2)
    }

    pub fn build(self) -> u32 {
        let Self { opcode, rd, funct3, rs1, rs2, funct7, imm } = self;
        match opcode & 0x7F {
            OP_REG | OP_REG_32 | OP_AMO => RType { opcode, funct3, funct7, rd, rs1, rs2 }.encode(),
            OP_IMM | OP_IMM_32 | OP_LOAD | OP_JALR | OP_SYSTEM => {
                IType { opcode, funct3, rd, rs1, imm }.encode()
            }
            OP_STORE => SType { opcode, funct3, rs1, rs2, imm }.encode(),
            OP_BRANCH => BType { opcode, funct3, rs1, rs2, offset: imm }.encode(),
            // The builder takes a U-type's raw 20-bit immediate, not shifted.
            OP_LUI | OP_AUIPC => UType { opcode, rd, imm20: imm.cast_unsigned() }.encode(),
            OP_JAL => JType { opcode, rd, offset: imm }.encode(),
            other => panic!("Unsupported opcode: {other:#x}"),
        }
    }
}
