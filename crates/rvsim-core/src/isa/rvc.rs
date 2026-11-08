//! Compressed Instruction Expansion.
//!
//! Provides the `expand` function which converts a 16-bit compressed instruction
//! into its 32-bit uncompressed equivalent.

use crate::isa::bits::sign_extend;
use crate::isa::encode::{BType, IType, JType, RType, SType, UType};
use crate::isa::encoding::privileged as sys_ops;
use crate::isa::encoding::rv64f::opcodes as fp_opcodes;
use crate::isa::encoding::rv64i::{funct3, funct7, opcodes};
use crate::isa::encoding::rvc::{QUADRANT_0, QUADRANT_1, QUADRANT_2, QUADRANT_MASK, q0, q1, q2};
use crate::isa::reg::{REG_RA, REG_SP, REG_ZERO, RegIdx};

/// Expands a 16-bit RVC instruction into its 32-bit equivalent, or `None`
/// when it is reserved or illegal.
pub const fn expand(inst: u16) -> Option<u32> {
    let funct3 = (inst >> 13) & 0x7;
    match inst & QUADRANT_MASK {
        QUADRANT_0 => expand_quadrant_0(inst, funct3),
        QUADRANT_1 => expand_quadrant_1(inst, funct3),
        QUADRANT_2 => expand_quadrant_2(inst, funct3),
        _ => None,
    }
}

/// Quadrant 0: the stack-pointer add and the loads and stores through a
/// register x8–x15.
const fn expand_quadrant_0(inst: u16, funct3: u16) -> Option<u32> {
    let (rs1, rd_or_rs2) = (reg_prime(inst, 7), reg_prime(inst, 2));
    Some(match funct3 {
        q0::C_ADDI4SPN => {
            let imm = addi4spn_imm(inst);
            if imm == 0 {
                return None;
            }
            addi(rd_or_rs2, REG_SP, imm)
        }
        q0::C_FLD => load(fp_opcodes::OP_LOAD_FP, funct3::LD, rd_or_rs2, rs1, double_offset(inst)),
        q0::C_LW => load(opcodes::OP_LOAD, funct3::LW, rd_or_rs2, rs1, word_offset(inst)),
        q0::C_LD => load(opcodes::OP_LOAD, funct3::LD, rd_or_rs2, rs1, double_offset(inst)),
        q0::C_FSD => {
            store(fp_opcodes::OP_STORE_FP, funct3::SD, rd_or_rs2, rs1, double_offset(inst))
        }
        q0::C_SW => store(opcodes::OP_STORE, funct3::SW, rd_or_rs2, rs1, word_offset(inst)),
        q0::C_SD => store(opcodes::OP_STORE, funct3::SD, rd_or_rs2, rs1, double_offset(inst)),
        _ => return None,
    })
}

/// Quadrant 1: immediates, the arithmetic on x8–x15, jumps and branches.
const fn expand_quadrant_1(inst: u16, funct3: u16) -> Option<u32> {
    let rd = reg(inst, 7);
    Some(match funct3 {
        q1::C_ADDI => addi(rd, rd, ci_imm(inst)),
        q1::C_ADDIW => {
            if rd.is_zero() {
                return None;
            }
            IType {
                opcode: opcodes::OP_IMM_32,
                funct3: funct3::ADD_SUB,
                rd,
                rs1: rd,
                imm: ci_imm(inst),
            }
            .encode()
        }
        q1::C_LI => addi(rd, REG_ZERO, ci_imm(inst)),
        q1::C_LUI_ADDI16SP if rd.as_u8() == REG_SP.as_u8() => {
            let imm = addi16sp_imm(inst);
            if imm == 0 {
                return None;
            }
            addi(REG_SP, REG_SP, imm)
        }
        q1::C_LUI_ADDI16SP => {
            let imm = ci_imm(inst);
            if imm == 0 {
                return None;
            }
            UType { opcode: opcodes::OP_LUI, rd, imm20: imm.cast_unsigned() }.encode()
        }
        q1::C_MISC_ALU => return expand_misc_alu(inst),
        q1::C_J => {
            JType { opcode: opcodes::OP_JAL, rd: REG_ZERO, offset: cj_offset(inst) }.encode()
        }
        q1::C_BEQZ => branch_if_zero(funct3::BEQ, inst),
        q1::C_BNEZ => branch_if_zero(funct3::BNE, inst),
        _ => return None,
    })
}

/// Quadrant 1's arithmetic on a register x8–x15: the shifts and `andi` by
/// an immediate, and the register-register operations.
const fn expand_misc_alu(inst: u16) -> Option<u32> {
    let rd = reg_prime(inst, 7);
    Some(match (inst >> 10) & 0x3 {
        0 => shift_right_immediate(rd, funct7::DEFAULT, inst),
        1 => shift_right_immediate(rd, funct7::SRA, inst),
        2 => IType { opcode: opcodes::OP_IMM, funct3: funct3::AND, rd, rs1: rd, imm: ci_imm(inst) }
            .encode(),
        _ => return register_alu(rd, inst),
    })
}

/// `srli` or `srai` of `rd` by the CI shift amount; `funct7` picks which.
const fn shift_right_immediate(rd: RegIdx, funct7: u32, inst: u16) -> u32 {
    let imm = ((funct7 << 5) | ci_uimm(inst)).cast_signed();
    IType { opcode: opcodes::OP_IMM, funct3: funct3::SRL_SRA, rd, rs1: rd, imm }.encode()
}

/// `c.sub`, `c.xor`, `c.or`, `c.and`, `c.subw` and `c.addw` on `rd` and rs2'.
const fn register_alu(rd: RegIdx, inst: u16) -> Option<u32> {
    let (opcode, funct3, funct7) = match ((inst >> 12) & 1, (inst >> 5) & 0x3) {
        (0, 0) => (opcodes::OP_REG, funct3::ADD_SUB, funct7::SUB),
        (0, 1) => (opcodes::OP_REG, funct3::XOR, funct7::DEFAULT),
        (0, 2) => (opcodes::OP_REG, funct3::OR, funct7::DEFAULT),
        (0, 3) => (opcodes::OP_REG, funct3::AND, funct7::DEFAULT),
        (1, 0) => (opcodes::OP_REG_32, funct3::ADD_SUB, funct7::SUB),
        (1, 1) => (opcodes::OP_REG_32, funct3::ADD_SUB, funct7::DEFAULT),
        _ => return None,
    };
    Some(RType { opcode, funct3, funct7, rd, rs1: rd, rs2: reg_prime(inst, 2) }.encode())
}

/// Quadrant 2: the shift, the stack-relative loads and stores, and the
/// register moves, jumps and `ebreak`.
const fn expand_quadrant_2(inst: u16, funct3: u16) -> Option<u32> {
    let (rd, rs2) = (reg(inst, 7), reg(inst, 2));
    Some(match funct3 {
        // With rd = x0 or shamt = 0 it is a HINT, which runs as a no-op:
        // slli x0, x0, shamt writes nothing.
        q2::C_SLLI => IType {
            opcode: opcodes::OP_IMM,
            funct3: funct3::SLL,
            rd,
            rs1: rd,
            imm: ci_uimm(inst).cast_signed(),
        }
        .encode(),
        q2::C_FLDSP => load(fp_opcodes::OP_LOAD_FP, funct3::LD, rd, REG_SP, ldsp_offset(inst)),
        q2::C_LWSP if !rd.is_zero() => {
            load(opcodes::OP_LOAD, funct3::LW, rd, REG_SP, lwsp_offset(inst))
        }
        q2::C_LDSP if !rd.is_zero() => {
            load(opcodes::OP_LOAD, funct3::LD, rd, REG_SP, ldsp_offset(inst))
        }
        q2::C_MISC_ALU => return expand_jump_move_add(inst),
        q2::C_FSDSP => store(fp_opcodes::OP_STORE_FP, funct3::SD, rs2, REG_SP, sdsp_offset(inst)),
        q2::C_SWSP => store(opcodes::OP_STORE, funct3::SW, rs2, REG_SP, swsp_offset(inst)),
        q2::C_SDSP => store(opcodes::OP_STORE, funct3::SD, rs2, REG_SP, sdsp_offset(inst)),
        _ => return None,
    })
}

/// Quadrant 2's `c.jr`, `c.mv`, `c.ebreak`, `c.jalr` and `c.add`: bit 12
/// tells a jump from its linking form and `c.mv` from `c.add`, and a zero
/// rs2 marks the jumps.
const fn expand_jump_move_add(inst: u16) -> Option<u32> {
    let (rs1, rs2) = (reg(inst, 7), reg(inst, 2));
    Some(match ((inst >> 12) & 1, rs2.is_zero()) {
        (0, true) if rs1.is_zero() => return None,
        (0, true) => jalr(REG_ZERO, rs1),
        (0, false) => add(rs1, REG_ZERO, rs2),
        (_, true) if rs1.is_zero() => sys_ops::EBREAK,
        (_, true) => jalr(REG_RA, rs1),
        (_, false) => add(rs1, rs1, rs2),
    })
}

/// `jalr rd, 0(rs1)`.
const fn jalr(rd: RegIdx, rs1: RegIdx) -> u32 {
    IType { opcode: opcodes::OP_JALR, funct3: funct3::JALR, rd, rs1, imm: 0 }.encode()
}

/// `add rd, rs1, rs2`.
const fn add(rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> u32 {
    RType {
        opcode: opcodes::OP_REG,
        funct3: funct3::ADD_SUB,
        funct7: funct7::DEFAULT,
        rd,
        rs1,
        rs2,
    }
    .encode()
}

/// `addi rd, rs1, imm`.
const fn addi(rd: RegIdx, rs1: RegIdx, imm: i32) -> u32 {
    IType { opcode: opcodes::OP_IMM, funct3: funct3::ADD_SUB, rd, rs1, imm }.encode()
}

/// A load of `rd` from `offset(rs1)`.
const fn load(opcode: u32, funct3: u32, rd: RegIdx, rs1: RegIdx, offset: i32) -> u32 {
    IType { opcode, funct3, rd, rs1, imm: offset }.encode()
}

/// A store of `rs2` to `offset(rs1)`.
const fn store(opcode: u32, funct3: u32, rs2: RegIdx, rs1: RegIdx, offset: i32) -> u32 {
    SType { opcode, funct3, rs1, rs2, imm: offset }.encode()
}

/// `c.beqz` or `c.bnez`: a branch comparing rs1' with x0.
const fn branch_if_zero(funct3: u32, inst: u16) -> u32 {
    BType {
        opcode: opcodes::OP_BRANCH,
        funct3,
        rs1: reg_prime(inst, 7),
        rs2: REG_ZERO,
        offset: cb_offset(inst),
    }
    .encode()
}

/// Bits `hi..=lo` of `inst`, shifted down.
const fn field(inst: u16, hi: u32, lo: u32) -> u32 {
    ((inst as u32) >> lo) & ((1 << (hi - lo + 1)) - 1)
}

/// A full register field at bit `shift`.
const fn reg(inst: u16, shift: u32) -> RegIdx {
    RegIdx::new(field(inst, shift + 4, shift) as u8)
}

/// A compressed register field (rd', rs1', rs2') at bit `shift`, which
/// names x8–x15.
const fn reg_prime(inst: u16, shift: u32) -> RegIdx {
    RegIdx::new(8 + field(inst, shift + 2, shift) as u8)
}

/// The CI format's 6-bit immediate, `imm[5] = inst[12]` and
/// `imm[4:0] = inst[6:2]`, unsigned (a shift amount).
const fn ci_uimm(inst: u16) -> u32 {
    field(inst, 12, 12) << 5 | field(inst, 6, 2)
}

/// The CI format's 6-bit immediate, sign-extended.
const fn ci_imm(inst: u16) -> i32 {
    sign_extend(ci_uimm(inst) as u64, 6) as i32
}

/// `c.addi4spn`'s scaled, zero-extended immediate: `nzuimm[5:4|9:6|2|3]`.
const fn addi4spn_imm(inst: u16) -> i32 {
    (field(inst, 6, 6) << 2
        | field(inst, 5, 5) << 3
        | field(inst, 12, 11) << 4
        | field(inst, 10, 7) << 6)
        .cast_signed()
}

/// `c.addi16sp`'s immediate, `nzimm[9|4|6|8:7|5]`, sign-extended.
const fn addi16sp_imm(inst: u16) -> i32 {
    let imm = field(inst, 6, 6) << 4
        | field(inst, 2, 2) << 5
        | field(inst, 5, 5) << 6
        | field(inst, 4, 3) << 7
        | field(inst, 12, 12) << 9;
    sign_extend(imm as u64, 10) as i32
}

/// The CL and CS formats' word offset: `uimm[5:3|2|6]`.
const fn word_offset(inst: u16) -> i32 {
    (field(inst, 6, 6) << 2 | field(inst, 12, 10) << 3 | field(inst, 5, 5) << 6).cast_signed()
}

/// The CL and CS formats' doubleword offset: `uimm[5:3|7:6]`.
const fn double_offset(inst: u16) -> i32 {
    (field(inst, 12, 10) << 3 | field(inst, 6, 5) << 6).cast_signed()
}

/// `c.lwsp`'s offset: `uimm[5|4:2|7:6]`.
const fn lwsp_offset(inst: u16) -> i32 {
    (field(inst, 12, 12) << 5 | field(inst, 6, 4) << 2 | field(inst, 3, 2) << 6).cast_signed()
}

/// `c.ldsp` and `c.fldsp`'s offset: `uimm[5|4:3|8:6]`.
const fn ldsp_offset(inst: u16) -> i32 {
    (field(inst, 12, 12) << 5 | field(inst, 6, 5) << 3 | field(inst, 4, 2) << 6).cast_signed()
}

/// `c.swsp`'s offset: `uimm[5:2|7:6]`.
const fn swsp_offset(inst: u16) -> i32 {
    (field(inst, 12, 9) << 2 | field(inst, 8, 7) << 6).cast_signed()
}

/// `c.sdsp` and `c.fsdsp`'s offset: `uimm[5:3|8:6]`.
const fn sdsp_offset(inst: u16) -> i32 {
    (field(inst, 12, 10) << 3 | field(inst, 9, 7) << 6).cast_signed()
}

/// `c.j`'s offset, `imm[11|4|9:8|10|6|7|3:1|5]`, sign-extended.
const fn cj_offset(inst: u16) -> i32 {
    let imm = field(inst, 5, 3) << 1
        | field(inst, 11, 11) << 4
        | field(inst, 2, 2) << 5
        | field(inst, 7, 7) << 6
        | field(inst, 6, 6) << 7
        | field(inst, 10, 9) << 8
        | field(inst, 8, 8) << 10
        | field(inst, 12, 12) << 11;
    sign_extend(imm as u64, 12) as i32
}

/// The CB branches' offset, `imm[8|4:3|7:6|2:1|5]`, sign-extended.
const fn cb_offset(inst: u16) -> i32 {
    let imm = field(inst, 4, 3) << 1
        | field(inst, 11, 10) << 3
        | field(inst, 2, 2) << 5
        | field(inst, 6, 5) << 6
        | field(inst, 12, 12) << 8;
    sign_extend(imm as u64, 9) as i32
}
