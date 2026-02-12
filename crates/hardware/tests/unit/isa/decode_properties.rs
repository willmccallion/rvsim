//! Instruction Decode Properties — 100% Opcode Coverage.
//!
//! Verifies that `decode()` correctly extracts opcode, register fields,
//! function codes, and sign-extended immediates for every instruction
//! format in RV64GC.
//!
//! # Coverage Matrix
//!
//! - R-type:  OP_REG (I + M), OP_REG_32 (I + M), OP_FP, OP_AMO
//! - I-type:  OP_IMM, OP_IMM_32, OP_LOAD, OP_LOAD_FP, OP_JALR, OP_SYSTEM
//! - S-type:  OP_STORE, OP_STORE_FP
//! - B-type:  OP_BRANCH
//! - U-type:  OP_LUI, OP_AUIPC
//! - J-type:  OP_JAL
//! - R4-type: OP_FMADD, OP_FMSUB, OP_FNMADD, OP_FNMSUB

use riscv_core::isa::decode::decode;
use riscv_core::isa::instruction::InstructionBits;

// ──────────────────────────────────────────────────────────
// Encoding helpers (construct raw 32-bit instructions)
// ──────────────────────────────────────────────────────────

/// Encode an R-type instruction.
fn r_type(opcode: u32, rd: u32, funct3: u32, rs1: u32, rs2: u32, funct7: u32) -> u32 {
    (funct7 & 0x7F) << 25
        | (rs2 & 0x1F) << 20
        | (rs1 & 0x1F) << 15
        | (funct3 & 0x7) << 12
        | (rd & 0x1F) << 7
        | (opcode & 0x7F)
}

/// Encode an I-type instruction.
fn i_type(opcode: u32, rd: u32, funct3: u32, rs1: u32, imm: i32) -> u32 {
    let imm_bits = (imm as u32) & 0xFFF;
    imm_bits << 20 | (rs1 & 0x1F) << 15 | (funct3 & 0x7) << 12 | (rd & 0x1F) << 7 | (opcode & 0x7F)
}

/// Encode an S-type instruction.
fn s_type(opcode: u32, funct3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let v = imm as u32;
    let hi = (v >> 5) & 0x7F;
    let lo = v & 0x1F;
    hi << 25
        | (rs2 & 0x1F) << 20
        | (rs1 & 0x1F) << 15
        | (funct3 & 0x7) << 12
        | lo << 7
        | (opcode & 0x7F)
}

/// Encode a B-type instruction.
fn b_type(opcode: u32, funct3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let v = imm as u32;
    let bit12 = (v >> 12) & 1;
    let bits10_5 = (v >> 5) & 0x3F;
    let bits4_1 = (v >> 1) & 0xF;
    let bit11 = (v >> 11) & 1;
    bit12 << 31
        | bits10_5 << 25
        | (rs2 & 0x1F) << 20
        | (rs1 & 0x1F) << 15
        | (funct3 & 0x7) << 12
        | bits4_1 << 8
        | bit11 << 7
        | (opcode & 0x7F)
}

/// Encode a U-type instruction.
fn u_type(opcode: u32, rd: u32, imm20: u32) -> u32 {
    (imm20 & 0xFFFFF) << 12 | (rd & 0x1F) << 7 | (opcode & 0x7F)
}

/// Encode a J-type instruction.
fn j_type(opcode: u32, rd: u32, imm: i32) -> u32 {
    let v = imm as u32;
    let bit20 = (v >> 20) & 1;
    let bits10_1 = (v >> 1) & 0x3FF;
    let bit11 = (v >> 11) & 1;
    let bits19_12 = (v >> 12) & 0xFF;
    bit20 << 31
        | bits10_1 << 21
        | bit11 << 20
        | bits19_12 << 12
        | (rd & 0x1F) << 7
        | (opcode & 0x7F)
}

/// Encode an R4-type (FMA) instruction.
fn r4_type(opcode: u32, rd: u32, funct3: u32, rs1: u32, rs2: u32, rs3: u32, fmt: u32) -> u32 {
    (rs3 & 0x1F) << 27
        | (fmt & 0x3) << 25
        | (rs2 & 0x1F) << 20
        | (rs1 & 0x1F) << 15
        | (funct3 & 0x7) << 12
        | (rd & 0x1F) << 7
        | (opcode & 0x7F)
}

// ──────────────────────────────────────────────────────────
// Import opcode/funct constants
// ──────────────────────────────────────────────────────────

use riscv_core::isa::privileged::opcodes as sys_op;
use riscv_core::isa::rv64a::funct5 as a_f5;
use riscv_core::isa::rv64a::opcodes as a_op;
use riscv_core::isa::rv64d::funct7 as d_f7;
use riscv_core::isa::rv64f::funct3 as f_f3;
use riscv_core::isa::rv64f::funct7 as f_f7;
use riscv_core::isa::rv64f::opcodes as f_op;
use riscv_core::isa::rv64i::funct3 as i_f3;
use riscv_core::isa::rv64i::funct7 as i_f7;
use riscv_core::isa::rv64i::opcodes as i_op;
use riscv_core::isa::rv64m::funct3 as m_f3;
use riscv_core::isa::rv64m::opcodes as m_op;

// ══════════════════════════════════════════════════════════
// 1. InstructionBits trait — field extraction
// ══════════════════════════════════════════════════════════

#[test]
fn field_extraction_opcode() {
    let inst: u32 = 0b1010101_00000_00000_000_00000_0110011; // OP_REG = 0x33
    assert_eq!(inst.opcode(), i_op::OP_REG);
}

#[test]
fn field_extraction_rd() {
    // rd = 15 (0b01111), placed at bits 7-11
    let inst = r_type(i_op::OP_REG, 15, 0, 0, 0, 0);
    assert_eq!(inst.rd(), 15);
}

#[test]
fn field_extraction_rs1() {
    let inst = r_type(i_op::OP_REG, 0, 0, 23, 0, 0);
    assert_eq!(inst.rs1(), 23);
}

#[test]
fn field_extraction_rs2() {
    let inst = r_type(i_op::OP_REG, 0, 0, 0, 31, 0);
    assert_eq!(inst.rs2(), 31);
}

#[test]
fn field_extraction_rs3() {
    // rs3 = bits 27-31 (used in FMA instructions)
    let inst = r4_type(f_op::OP_FMADD, 1, 0, 2, 3, 17, 0);
    assert_eq!(inst.rs3(), 17);
}

#[test]
fn field_extraction_funct3() {
    let inst = r_type(i_op::OP_REG, 0, 5, 0, 0, 0);
    assert_eq!(inst.funct3(), 5);
}

#[test]
fn field_extraction_funct7() {
    let inst = r_type(i_op::OP_REG, 0, 0, 0, 0, 0b0100000);
    assert_eq!(inst.funct7(), 0b0100000);
}

#[test]
fn field_extraction_csr() {
    // CSR address occupies bits 20-31 (12 bits). For CSRRW x1, mstatus(0x300), x2:
    let inst = i_type(sys_op::OP_SYSTEM, 1, sys_op::CSRRW, 2, 0x300);
    assert_eq!(inst.csr(), 0x300);
}

#[test]
fn field_extraction_all_ones() {
    let inst: u32 = 0xFFFF_FFFF;
    assert_eq!(inst.opcode(), 0x7F);
    assert_eq!(inst.rd(), 31);
    assert_eq!(inst.funct3(), 7);
    assert_eq!(inst.rs1(), 31);
    assert_eq!(inst.rs2(), 31);
    assert_eq!(inst.funct7(), 0x7F);
    assert_eq!(inst.rs3(), 31);
    assert_eq!(inst.csr(), 0xFFF);
}

#[test]
fn field_extraction_all_zeros() {
    let inst: u32 = 0x0000_0000;
    assert_eq!(inst.opcode(), 0);
    assert_eq!(inst.rd(), 0);
    assert_eq!(inst.funct3(), 0);
    assert_eq!(inst.rs1(), 0);
    assert_eq!(inst.rs2(), 0);
    assert_eq!(inst.funct7(), 0);
    assert_eq!(inst.rs3(), 0);
    assert_eq!(inst.csr(), 0);
}

// ══════════════════════════════════════════════════════════
// 2. R-Type: RV64I base register-register
// ══════════════════════════════════════════════════════════

#[test]
fn decode_r_type_add() {
    let inst = r_type(i_op::OP_REG, 5, i_f3::ADD_SUB, 10, 15, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.rd, 5);
    assert_eq!(d.rs1, 10);
    assert_eq!(d.rs2, 15);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.funct7, i_f7::DEFAULT);
    assert_eq!(d.imm, 0, "R-type has no immediate");
}

#[test]
fn decode_r_type_sub() {
    let inst = r_type(i_op::OP_REG, 1, i_f3::ADD_SUB, 2, 3, i_f7::SUB);
    let d = decode(inst);
    assert_eq!(d.funct7, i_f7::SUB);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
}

#[test]
fn decode_r_type_sll() {
    let inst = r_type(i_op::OP_REG, 1, i_f3::SLL, 2, 3, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SLL);
}

#[test]
fn decode_r_type_slt() {
    let inst = r_type(i_op::OP_REG, 1, i_f3::SLT, 2, 3, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SLT);
}

#[test]
fn decode_r_type_sltu() {
    let inst = r_type(i_op::OP_REG, 1, i_f3::SLTU, 2, 3, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SLTU);
}

#[test]
fn decode_r_type_xor() {
    let inst = r_type(i_op::OP_REG, 1, i_f3::XOR, 2, 3, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::XOR);
}

#[test]
fn decode_r_type_srl() {
    let inst = r_type(i_op::OP_REG, 1, i_f3::SRL_SRA, 2, 3, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SRL_SRA);
    assert_eq!(d.funct7, i_f7::DEFAULT);
}

#[test]
fn decode_r_type_sra() {
    let inst = r_type(i_op::OP_REG, 1, i_f3::SRL_SRA, 2, 3, i_f7::SRA);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SRL_SRA);
    assert_eq!(d.funct7, i_f7::SRA);
}

#[test]
fn decode_r_type_or() {
    let inst = r_type(i_op::OP_REG, 1, i_f3::OR, 2, 3, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::OR);
}

#[test]
fn decode_r_type_and() {
    let inst = r_type(i_op::OP_REG, 1, i_f3::AND, 2, 3, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::AND);
}

// ══════════════════════════════════════════════════════════
// 3. R-Type: RV64I word operations (OP_REG_32)
// ══════════════════════════════════════════════════════════

#[test]
fn decode_r_type_addw() {
    let inst = r_type(i_op::OP_REG_32, 5, i_f3::ADD_SUB, 10, 15, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct7, i_f7::DEFAULT);
}

#[test]
fn decode_r_type_subw() {
    let inst = r_type(i_op::OP_REG_32, 5, i_f3::ADD_SUB, 10, 15, i_f7::SUB);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct7, i_f7::SUB);
}

#[test]
fn decode_r_type_sllw() {
    let inst = r_type(i_op::OP_REG_32, 1, i_f3::SLL, 2, 3, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct3, i_f3::SLL);
}

#[test]
fn decode_r_type_srlw() {
    let inst = r_type(i_op::OP_REG_32, 1, i_f3::SRL_SRA, 2, 3, i_f7::DEFAULT);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct3, i_f3::SRL_SRA);
    assert_eq!(d.funct7, i_f7::DEFAULT);
}

#[test]
fn decode_r_type_sraw() {
    let inst = r_type(i_op::OP_REG_32, 1, i_f3::SRL_SRA, 2, 3, i_f7::SRA);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct7, i_f7::SRA);
}

// ══════════════════════════════════════════════════════════
// 4. R-Type: M-extension (multiply/divide)
// ══════════════════════════════════════════════════════════

#[test]
fn decode_r_type_mul() {
    let inst = r_type(i_op::OP_REG, 1, m_f3::MUL, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct7, m_op::M_EXTENSION);
    assert_eq!(d.funct3, m_f3::MUL);
}

#[test]
fn decode_r_type_mulh() {
    let inst = r_type(i_op::OP_REG, 1, m_f3::MULH, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct3, m_f3::MULH);
}

#[test]
fn decode_r_type_mulhsu() {
    let inst = r_type(i_op::OP_REG, 1, m_f3::MULHSU, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct3, m_f3::MULHSU);
}

#[test]
fn decode_r_type_mulhu() {
    let inst = r_type(i_op::OP_REG, 1, m_f3::MULHU, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct3, m_f3::MULHU);
}

#[test]
fn decode_r_type_div() {
    let inst = r_type(i_op::OP_REG, 1, m_f3::DIV, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct3, m_f3::DIV);
}

#[test]
fn decode_r_type_divu() {
    let inst = r_type(i_op::OP_REG, 1, m_f3::DIVU, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct3, m_f3::DIVU);
}

#[test]
fn decode_r_type_rem() {
    let inst = r_type(i_op::OP_REG, 1, m_f3::REM, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct3, m_f3::REM);
}

#[test]
fn decode_r_type_remu() {
    let inst = r_type(i_op::OP_REG, 1, m_f3::REMU, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct3, m_f3::REMU);
}

// M-extension word variants (OP_REG_32 + funct7=1)
#[test]
fn decode_r_type_mulw() {
    let inst = r_type(i_op::OP_REG_32, 1, m_f3::MUL, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct7, m_op::M_EXTENSION);
    assert_eq!(d.funct3, m_f3::MUL);
}

#[test]
fn decode_r_type_divw() {
    let inst = r_type(i_op::OP_REG_32, 1, m_f3::DIV, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct3, m_f3::DIV);
}

#[test]
fn decode_r_type_divuw() {
    let inst = r_type(i_op::OP_REG_32, 1, m_f3::DIVU, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct3, m_f3::DIVU);
}

#[test]
fn decode_r_type_remw() {
    let inst = r_type(i_op::OP_REG_32, 1, m_f3::REM, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct3, m_f3::REM);
}

#[test]
fn decode_r_type_remuw() {
    let inst = r_type(i_op::OP_REG_32, 1, m_f3::REMU, 2, 3, m_op::M_EXTENSION);
    let d = decode(inst);
    assert_eq!(d.funct3, m_f3::REMU);
}

// ══════════════════════════════════════════════════════════
// 5. I-Type: OP_IMM (ADDI, SLTI, SLTIU, XORI, ORI, ANDI)
// ══════════════════════════════════════════════════════════

#[test]
fn decode_i_type_addi_positive() {
    let inst = i_type(i_op::OP_IMM, 5, i_f3::ADD_SUB, 10, 42);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.rd, 5);
    assert_eq!(d.rs1, 10);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.imm, 42);
}

#[test]
fn decode_i_type_addi_negative() {
    let inst = i_type(i_op::OP_IMM, 1, i_f3::ADD_SUB, 2, -1);
    let d = decode(inst);
    assert_eq!(d.imm, -1, "I-type immediate must sign-extend -1");
}

#[test]
fn decode_i_type_addi_max_positive() {
    // Max positive 12-bit: 2047
    let inst = i_type(i_op::OP_IMM, 1, i_f3::ADD_SUB, 2, 2047);
    let d = decode(inst);
    assert_eq!(d.imm, 2047);
}

#[test]
fn decode_i_type_addi_min_negative() {
    // Min negative 12-bit: -2048
    let inst = i_type(i_op::OP_IMM, 1, i_f3::ADD_SUB, 2, -2048);
    let d = decode(inst);
    assert_eq!(d.imm, -2048);
}

#[test]
fn decode_i_type_slti() {
    let inst = i_type(i_op::OP_IMM, 1, i_f3::SLT, 2, -5);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SLT);
    assert_eq!(d.imm, -5);
}

#[test]
fn decode_i_type_sltiu() {
    let inst = i_type(i_op::OP_IMM, 1, i_f3::SLTU, 2, 100);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SLTU);
    assert_eq!(d.imm, 100);
}

#[test]
fn decode_i_type_xori() {
    let inst = i_type(i_op::OP_IMM, 1, i_f3::XOR, 2, -1);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::XOR);
    assert_eq!(d.imm, -1);
}

#[test]
fn decode_i_type_ori() {
    let inst = i_type(i_op::OP_IMM, 1, i_f3::OR, 2, 0xFF);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::OR);
    assert_eq!(d.imm, 0xFF);
}

#[test]
fn decode_i_type_andi() {
    let inst = i_type(i_op::OP_IMM, 1, i_f3::AND, 2, 0x3F);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::AND);
    assert_eq!(d.imm, 0x3F);
}

// Shift immediates (encoded in I-type with shamt in lower bits)
#[test]
fn decode_i_type_slli() {
    // SLLI rd, rs1, shamt: funct7=0, shamt in imm[5:0]
    let inst = i_type(i_op::OP_IMM, 1, i_f3::SLL, 2, 13);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SLL);
    assert_eq!(d.imm & 0x3F, 13);
}

#[test]
fn decode_i_type_srli() {
    let inst = i_type(i_op::OP_IMM, 1, i_f3::SRL_SRA, 2, 7);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SRL_SRA);
    assert_eq!(d.imm & 0x3F, 7);
}

#[test]
fn decode_i_type_srai() {
    // SRAI has bit 30 set (funct7 = 0b0100000 → imm[11:5] = 0x20)
    let imm = (0b0100000 << 5) | 3; // shamt=3
    let inst = i_type(i_op::OP_IMM, 1, i_f3::SRL_SRA, 2, imm);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SRL_SRA);
    assert_eq!(d.imm & 0x3F, 3);
}

// ══════════════════════════════════════════════════════════
// 6. I-Type: OP_IMM_32 (ADDIW, SLLIW, SRLIW, SRAIW)
// ══════════════════════════════════════════════════════════

#[test]
fn decode_i_type_addiw() {
    let inst = i_type(i_op::OP_IMM_32, 1, i_f3::ADD_SUB, 2, -100);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_IMM_32);
    assert_eq!(d.imm, -100);
}

#[test]
fn decode_i_type_slliw() {
    let inst = i_type(i_op::OP_IMM_32, 1, i_f3::SLL, 2, 5);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_IMM_32);
    assert_eq!(d.funct3, i_f3::SLL);
}

// ══════════════════════════════════════════════════════════
// 7. I-Type: Loads (OP_LOAD)
// ══════════════════════════════════════════════════════════

#[test]
fn decode_load_lb() {
    let inst = i_type(i_op::OP_LOAD, 1, i_f3::LB, 2, -8);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_LOAD);
    assert_eq!(d.funct3, i_f3::LB);
    assert_eq!(d.imm, -8);
}

#[test]
fn decode_load_lh() {
    let inst = i_type(i_op::OP_LOAD, 1, i_f3::LH, 2, 16);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::LH);
}

#[test]
fn decode_load_lw() {
    let inst = i_type(i_op::OP_LOAD, 1, i_f3::LW, 2, 128);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::LW);
    assert_eq!(d.imm, 128);
}

#[test]
fn decode_load_ld() {
    let inst = i_type(i_op::OP_LOAD, 1, i_f3::LD, 2, 256);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::LD);
}

#[test]
fn decode_load_lbu() {
    let inst = i_type(i_op::OP_LOAD, 1, i_f3::LBU, 2, 0);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::LBU);
}

#[test]
fn decode_load_lhu() {
    let inst = i_type(i_op::OP_LOAD, 1, i_f3::LHU, 2, 4);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::LHU);
}

#[test]
fn decode_load_lwu() {
    let inst = i_type(i_op::OP_LOAD, 1, i_f3::LWU, 2, 8);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::LWU);
}

// FP loads
#[test]
fn decode_load_flw() {
    let inst = i_type(f_op::OP_LOAD_FP, 1, i_f3::LW, 2, 64);
    let d = decode(inst);
    assert_eq!(d.opcode, f_op::OP_LOAD_FP);
    assert_eq!(d.imm, 64);
}

#[test]
fn decode_load_fld() {
    let inst = i_type(f_op::OP_LOAD_FP, 1, i_f3::LD, 2, -32);
    let d = decode(inst);
    assert_eq!(d.opcode, f_op::OP_LOAD_FP);
    assert_eq!(d.imm, -32);
}

// ══════════════════════════════════════════════════════════
// 8. S-Type: Stores (OP_STORE, OP_STORE_FP)
// ══════════════════════════════════════════════════════════

#[test]
fn decode_store_sb() {
    let inst = s_type(i_op::OP_STORE, i_f3::SB, 2, 3, 7);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_STORE);
    assert_eq!(d.funct3, i_f3::SB);
    assert_eq!(d.rs1, 2);
    assert_eq!(d.rs2, 3);
    assert_eq!(d.imm, 7);
}

#[test]
fn decode_store_sh() {
    let inst = s_type(i_op::OP_STORE, i_f3::SH, 2, 3, -4);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SH);
    assert_eq!(d.imm, -4);
}

#[test]
fn decode_store_sw() {
    let inst = s_type(i_op::OP_STORE, i_f3::SW, 2, 3, 100);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SW);
    assert_eq!(d.imm, 100);
}

#[test]
fn decode_store_sd() {
    let inst = s_type(i_op::OP_STORE, i_f3::SD, 2, 3, -2048);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::SD);
    assert_eq!(d.imm, -2048);
}

#[test]
fn decode_store_sd_max_positive() {
    let inst = s_type(i_op::OP_STORE, i_f3::SD, 2, 3, 2047);
    let d = decode(inst);
    assert_eq!(d.imm, 2047);
}

#[test]
fn decode_store_fsw() {
    let inst = s_type(f_op::OP_STORE_FP, i_f3::SW, 2, 3, 48);
    let d = decode(inst);
    assert_eq!(d.opcode, f_op::OP_STORE_FP);
    assert_eq!(d.imm, 48);
}

#[test]
fn decode_store_fsd() {
    let inst = s_type(f_op::OP_STORE_FP, i_f3::SD, 2, 3, -16);
    let d = decode(inst);
    assert_eq!(d.opcode, f_op::OP_STORE_FP);
    assert_eq!(d.imm, -16);
}

// ══════════════════════════════════════════════════════════
// 9. B-Type: Branches
// ══════════════════════════════════════════════════════════

#[test]
fn decode_branch_beq() {
    let inst = b_type(i_op::OP_BRANCH, i_f3::BEQ, 5, 6, 64);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_BRANCH);
    assert_eq!(d.funct3, i_f3::BEQ);
    assert_eq!(d.rs1, 5);
    assert_eq!(d.rs2, 6);
    assert_eq!(d.imm, 64);
}

#[test]
fn decode_branch_bne() {
    let inst = b_type(i_op::OP_BRANCH, i_f3::BNE, 1, 2, -8);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::BNE);
    assert_eq!(d.imm, -8);
}

#[test]
fn decode_branch_blt() {
    let inst = b_type(i_op::OP_BRANCH, i_f3::BLT, 1, 2, 128);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::BLT);
    assert_eq!(d.imm, 128);
}

#[test]
fn decode_branch_bge() {
    let inst = b_type(i_op::OP_BRANCH, i_f3::BGE, 1, 2, -256);
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::BGE);
    assert_eq!(d.imm, -256);
}

#[test]
fn decode_branch_bltu() {
    let inst = b_type(i_op::OP_BRANCH, i_f3::BLTU, 1, 2, 4096 - 2); // max positive even offset
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::BLTU);
    assert_eq!(d.imm, 4094);
}

#[test]
fn decode_branch_bgeu() {
    let inst = b_type(i_op::OP_BRANCH, i_f3::BGEU, 1, 2, -4096); // min negative offset
    let d = decode(inst);
    assert_eq!(d.funct3, i_f3::BGEU);
    assert_eq!(d.imm, -4096);
}

// ══════════════════════════════════════════════════════════
// 10. U-Type: LUI, AUIPC
// ══════════════════════════════════════════════════════════

#[test]
fn decode_lui() {
    let inst = u_type(i_op::OP_LUI, 5, 0xDEADB);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_LUI);
    assert_eq!(d.rd, 5);
    // U-type imm = upper 20 bits << 12 (sign-extended to i64)
    assert_eq!(d.imm, 0xDEADB000u32 as i32 as i64);
}

#[test]
fn decode_auipc() {
    let inst = u_type(i_op::OP_AUIPC, 10, 0x00001);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_AUIPC);
    assert_eq!(d.rd, 10);
    assert_eq!(d.imm, 0x1000);
}

#[test]
fn decode_lui_sign_extension() {
    // imm20 = 0x80000 → top bit set → should sign-extend to negative
    let inst = u_type(i_op::OP_LUI, 1, 0x80000);
    let d = decode(inst);
    assert_eq!(d.imm, 0x80000000u32 as i32 as i64);
    assert!(
        d.imm < 0,
        "U-type with bit 31 set must sign-extend to negative"
    );
}

// ══════════════════════════════════════════════════════════
// 11. J-Type: JAL
// ══════════════════════════════════════════════════════════

#[test]
fn decode_jal_positive() {
    let inst = j_type(i_op::OP_JAL, 1, 100);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_JAL);
    assert_eq!(d.rd, 1);
    assert_eq!(d.imm, 100);
}

#[test]
fn decode_jal_negative() {
    let inst = j_type(i_op::OP_JAL, 1, -20);
    let d = decode(inst);
    assert_eq!(d.imm, -20);
}

#[test]
fn decode_jal_large_positive() {
    // Max positive: 2^20 - 2 = 1048574
    let inst = j_type(i_op::OP_JAL, 1, 0xFFFFE);
    let d = decode(inst);
    assert_eq!(d.imm, 0xFFFFE);
}

#[test]
fn decode_jal_max_negative() {
    // Min negative: -2^20 = -1048576
    let inst = j_type(i_op::OP_JAL, 0, -1048576);
    let d = decode(inst);
    assert_eq!(d.imm, -1048576);
}

// ══════════════════════════════════════════════════════════
// 12. I-Type: JALR
// ══════════════════════════════════════════════════════════

#[test]
fn decode_jalr() {
    let inst = i_type(i_op::OP_JALR, 1, 0, 5, 8);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_JALR);
    assert_eq!(d.rd, 1);
    assert_eq!(d.rs1, 5);
    assert_eq!(d.imm, 8);
}

#[test]
fn decode_jalr_negative() {
    let inst = i_type(i_op::OP_JALR, 0, 0, 1, -4);
    let d = decode(inst);
    assert_eq!(d.imm, -4);
}

// ══════════════════════════════════════════════════════════
// 13. R-Type: Floating-point arithmetic (OP_FP)
// ══════════════════════════════════════════════════════════

#[test]
fn decode_fp_fadd_s() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 3, f_f7::FADD);
    let d = decode(inst);
    assert_eq!(d.opcode, f_op::OP_FP);
    assert_eq!(d.funct7, f_f7::FADD);
}

#[test]
fn decode_fp_fsub_s() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 3, f_f7::FSUB);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FSUB);
}

#[test]
fn decode_fp_fmul_s() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 3, f_f7::FMUL);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FMUL);
}

#[test]
fn decode_fp_fdiv_s() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 3, f_f7::FDIV);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FDIV);
}

#[test]
fn decode_fp_fsqrt_s() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 0, f_f7::FSQRT);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FSQRT);
}

#[test]
fn decode_fp_fsgnj_s() {
    let inst = r_type(f_op::OP_FP, 1, f_f3::FSGNJ, 2, 3, f_f7::FSGNJ);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FSGNJ);
    assert_eq!(d.funct3, f_f3::FSGNJ);
}

#[test]
fn decode_fp_fmin_s() {
    let inst = r_type(f_op::OP_FP, 1, f_f3::FMIN, 2, 3, f_f7::FMIN_MAX);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FMIN_MAX);
    assert_eq!(d.funct3, f_f3::FMIN);
}

#[test]
fn decode_fp_fmax_s() {
    let inst = r_type(f_op::OP_FP, 1, f_f3::FMAX, 2, 3, f_f7::FMIN_MAX);
    let d = decode(inst);
    assert_eq!(d.funct3, f_f3::FMAX);
}

#[test]
fn decode_fp_feq() {
    let inst = r_type(f_op::OP_FP, 1, f_f3::FEQ, 2, 3, f_f7::FCMP);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FCMP);
    assert_eq!(d.funct3, f_f3::FEQ);
}

#[test]
fn decode_fp_flt() {
    let inst = r_type(f_op::OP_FP, 1, f_f3::FLT, 2, 3, f_f7::FCMP);
    let d = decode(inst);
    assert_eq!(d.funct3, f_f3::FLT);
}

#[test]
fn decode_fp_fle() {
    let inst = r_type(f_op::OP_FP, 1, f_f3::FLE, 2, 3, f_f7::FCMP);
    let d = decode(inst);
    assert_eq!(d.funct3, f_f3::FLE);
}

#[test]
fn decode_fp_fclass() {
    let inst = r_type(f_op::OP_FP, 1, f_f3::FCLASS, 2, 0, f_f7::FCLASS_MV_X_F);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FCLASS_MV_X_F);
    assert_eq!(d.funct3, f_f3::FCLASS);
}

#[test]
fn decode_fp_fcvt_w_s() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 0, f_f7::FCVT_W_F);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FCVT_W_F);
}

#[test]
fn decode_fp_fcvt_s_w() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 0, f_f7::FCVT_F_W);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FCVT_F_W);
}

#[test]
fn decode_fp_fmv_x_w() {
    let inst = r_type(f_op::OP_FP, 1, f_f3::FMV_X_W, 2, 0, f_f7::FCLASS_MV_X_F);
    let d = decode(inst);
    assert_eq!(d.funct3, f_f3::FMV_X_W);
}

#[test]
fn decode_fp_fmv_w_x() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 0, f_f7::FMV_F_X);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FMV_F_X);
}

// Double-precision variants
#[test]
fn decode_fp_fadd_d() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 3, d_f7::FADD_D);
    let d = decode(inst);
    assert_eq!(d.funct7, d_f7::FADD_D);
}

#[test]
fn decode_fp_fsub_d() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 3, d_f7::FSUB_D);
    let d = decode(inst);
    assert_eq!(d.funct7, d_f7::FSUB_D);
}

#[test]
fn decode_fp_fmul_d() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 3, d_f7::FMUL_D);
    let d = decode(inst);
    assert_eq!(d.funct7, d_f7::FMUL_D);
}

#[test]
fn decode_fp_fdiv_d() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 3, d_f7::FDIV_D);
    let d = decode(inst);
    assert_eq!(d.funct7, d_f7::FDIV_D);
}

#[test]
fn decode_fp_fcvt_s_d() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 0, d_f7::FCVT_S_D);
    let d = decode(inst);
    assert_eq!(d.funct7, d_f7::FCVT_S_D);
}

#[test]
fn decode_fp_fcvt_d_s() {
    let inst = r_type(f_op::OP_FP, 1, 0, 2, 0, f_f7::FCVT_DS);
    let d = decode(inst);
    assert_eq!(d.funct7, f_f7::FCVT_DS);
}

// ══════════════════════════════════════════════════════════
// 14. R4-Type: FMA (FMADD, FMSUB, FNMADD, FNMSUB)
// ══════════════════════════════════════════════════════════

#[test]
fn decode_fmadd_s() {
    let inst = r4_type(f_op::OP_FMADD, 1, 0, 2, 3, 4, 0b00);
    let d = decode(inst);
    assert_eq!(d.opcode, f_op::OP_FMADD);
    assert_eq!(d.rd, 1);
    assert_eq!(d.rs1, 2);
    assert_eq!(d.rs2, 3);
    assert_eq!(inst.rs3(), 4);
}

#[test]
fn decode_fmsub_s() {
    let inst = r4_type(f_op::OP_FMSUB, 5, 0, 6, 7, 8, 0b00);
    let d = decode(inst);
    assert_eq!(d.opcode, f_op::OP_FMSUB);
}

#[test]
fn decode_fnmsub_s() {
    let inst = r4_type(f_op::OP_FNMSUB, 1, 0, 2, 3, 4, 0b00);
    let d = decode(inst);
    assert_eq!(d.opcode, f_op::OP_FNMSUB);
}

#[test]
fn decode_fnmadd_s() {
    let inst = r4_type(f_op::OP_FNMADD, 1, 0, 2, 3, 4, 0b00);
    let d = decode(inst);
    assert_eq!(d.opcode, f_op::OP_FNMADD);
}

#[test]
fn decode_fmadd_d() {
    let inst = r4_type(f_op::OP_FMADD, 1, 0, 2, 3, 4, 0b01);
    let d = decode(inst);
    assert_eq!(d.opcode, f_op::OP_FMADD);
    // fmt=01 distinguishes D from S in the funct7 lower bits
}

// ══════════════════════════════════════════════════════════
// 15. Atomic (OP_AMO)
// ══════════════════════════════════════════════════════════

/// Encode an AMO instruction: funct5 | aq | rl | rs2 | rs1 | funct3 | rd | opcode
fn amo_type(funct5: u32, aq: bool, rl: bool, rs2: u32, rs1: u32, funct3: u32, rd: u32) -> u32 {
    (funct5 & 0x1F) << 27
        | (aq as u32) << 26
        | (rl as u32) << 25
        | (rs2 & 0x1F) << 20
        | (rs1 & 0x1F) << 15
        | (funct3 & 0x7) << 12
        | (rd & 0x1F) << 7
        | a_op::OP_AMO
}

#[test]
fn decode_amo_lr_w() {
    let inst = amo_type(a_f5::LR, true, false, 0, 5, 0b010, 1);
    let d = decode(inst);
    assert_eq!(d.opcode, a_op::OP_AMO);
    assert_eq!(d.rs1, 5);
    assert_eq!(d.rd, 1);
    // funct5 is in upper bits of funct7
    assert_eq!(d.funct7 >> 2, a_f5::LR);
}

#[test]
fn decode_amo_sc_w() {
    let inst = amo_type(a_f5::SC, false, true, 3, 5, 0b010, 1);
    let d = decode(inst);
    assert_eq!(d.funct7 >> 2, a_f5::SC);
    assert_eq!(d.rs2, 3);
}

#[test]
fn decode_amo_swap_d() {
    let inst = amo_type(a_f5::AMOSWAP, true, true, 3, 5, 0b011, 1);
    let d = decode(inst);
    assert_eq!(d.funct7 >> 2, a_f5::AMOSWAP);
    assert_eq!(d.funct3, 0b011); // 64-bit
}

#[test]
fn decode_amo_add() {
    let inst = amo_type(a_f5::AMOADD, false, false, 3, 5, 0b010, 1);
    let d = decode(inst);
    assert_eq!(d.funct7 >> 2, a_f5::AMOADD);
}

#[test]
fn decode_amo_xor() {
    let inst = amo_type(a_f5::AMOXOR, false, false, 3, 5, 0b010, 1);
    let d = decode(inst);
    assert_eq!(d.funct7 >> 2, a_f5::AMOXOR);
}

#[test]
fn decode_amo_and() {
    let inst = amo_type(a_f5::AMOAND, false, false, 3, 5, 0b010, 1);
    let d = decode(inst);
    assert_eq!(d.funct7 >> 2, a_f5::AMOAND);
}

#[test]
fn decode_amo_or() {
    let inst = amo_type(a_f5::AMOOR, false, false, 3, 5, 0b010, 1);
    let d = decode(inst);
    assert_eq!(d.funct7 >> 2, a_f5::AMOOR);
}

#[test]
fn decode_amo_min() {
    let inst = amo_type(a_f5::AMOMIN, false, false, 3, 5, 0b010, 1);
    let d = decode(inst);
    assert_eq!(d.funct7 >> 2, a_f5::AMOMIN);
}

#[test]
fn decode_amo_max() {
    let inst = amo_type(a_f5::AMOMAX, false, false, 3, 5, 0b010, 1);
    let d = decode(inst);
    assert_eq!(d.funct7 >> 2, a_f5::AMOMAX);
}

#[test]
fn decode_amo_minu() {
    let inst = amo_type(a_f5::AMOMINU, false, false, 3, 5, 0b010, 1);
    let d = decode(inst);
    assert_eq!(d.funct7 >> 2, a_f5::AMOMINU);
}

#[test]
fn decode_amo_maxu() {
    let inst = amo_type(a_f5::AMOMAXU, false, false, 3, 5, 0b010, 1);
    let d = decode(inst);
    assert_eq!(d.funct7 >> 2, a_f5::AMOMAXU);
}

// ══════════════════════════════════════════════════════════
// 16. System instructions
// ══════════════════════════════════════════════════════════

#[test]
fn decode_ecall() {
    let d = decode(sys_op::ECALL);
    assert_eq!(d.opcode, sys_op::OP_SYSTEM);
    assert_eq!(d.raw, sys_op::ECALL);
}

#[test]
fn decode_ebreak() {
    let d = decode(sys_op::EBREAK);
    assert_eq!(d.opcode, sys_op::OP_SYSTEM);
    assert_eq!(d.raw, sys_op::EBREAK);
}

#[test]
fn decode_mret() {
    let d = decode(sys_op::MRET);
    assert_eq!(d.opcode, sys_op::OP_SYSTEM);
}

#[test]
fn decode_sret() {
    let d = decode(sys_op::SRET);
    assert_eq!(d.opcode, sys_op::OP_SYSTEM);
}

#[test]
fn decode_wfi() {
    let d = decode(sys_op::WFI);
    assert_eq!(d.opcode, sys_op::OP_SYSTEM);
}

#[test]
fn decode_csrrw() {
    let inst = i_type(sys_op::OP_SYSTEM, 1, sys_op::CSRRW, 2, 0x300);
    let d = decode(inst);
    assert_eq!(d.opcode, sys_op::OP_SYSTEM);
    assert_eq!(d.funct3, sys_op::CSRRW);
    assert_eq!(d.rd, 1);
    assert_eq!(d.rs1, 2);
}

#[test]
fn decode_csrrs() {
    let inst = i_type(sys_op::OP_SYSTEM, 1, sys_op::CSRRS, 2, 0x300);
    let d = decode(inst);
    assert_eq!(d.funct3, sys_op::CSRRS);
}

#[test]
fn decode_csrrc() {
    let inst = i_type(sys_op::OP_SYSTEM, 1, sys_op::CSRRC, 2, 0x300);
    let d = decode(inst);
    assert_eq!(d.funct3, sys_op::CSRRC);
}

#[test]
fn decode_csrrwi() {
    let inst = i_type(sys_op::OP_SYSTEM, 1, sys_op::CSRRWI, 5, 0x300);
    let d = decode(inst);
    assert_eq!(d.funct3, sys_op::CSRRWI);
    assert_eq!(d.rs1, 5); // zimm[4:0] in rs1 field
}

#[test]
fn decode_csrrsi() {
    let inst = i_type(sys_op::OP_SYSTEM, 1, sys_op::CSRRSI, 3, 0x344);
    let d = decode(inst);
    assert_eq!(d.funct3, sys_op::CSRRSI);
}

#[test]
fn decode_csrrci() {
    let inst = i_type(sys_op::OP_SYSTEM, 1, sys_op::CSRRCI, 7, 0x305);
    let d = decode(inst);
    assert_eq!(d.funct3, sys_op::CSRRCI);
}

// ══════════════════════════════════════════════════════════
// 17. FENCE / FENCE.I
// ══════════════════════════════════════════════════════════

#[test]
fn decode_fence() {
    let inst = i_type(i_op::OP_MISC_MEM, 0, i_f3::FENCE, 0, 0x0FF);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_MISC_MEM);
    assert_eq!(d.funct3, i_f3::FENCE);
}

#[test]
fn decode_fence_i() {
    let inst = i_type(i_op::OP_MISC_MEM, 0, i_f3::FENCE_I, 0, 0);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_MISC_MEM);
    assert_eq!(d.funct3, i_f3::FENCE_I);
}

// ══════════════════════════════════════════════════════════
// 18. Immediate edge-case round-trip properties
// ══════════════════════════════════════════════════════════

#[test]
fn i_type_imm_round_trip_all_values() {
    // Verify every 12-bit signed value round-trips through encode/decode.
    for raw in -2048i32..=2047 {
        let inst = i_type(i_op::OP_IMM, 0, 0, 0, raw);
        let d = decode(inst);
        assert_eq!(d.imm, raw as i64, "I-type round-trip failed for imm={raw}");
    }
}

#[test]
fn s_type_imm_round_trip_boundaries() {
    for &val in &[-2048i32, -1, 0, 1, 2047] {
        let inst = s_type(i_op::OP_STORE, 0, 0, 0, val);
        let d = decode(inst);
        assert_eq!(d.imm, val as i64, "S-type round-trip failed for imm={val}");
    }
}

#[test]
fn b_type_imm_round_trip_even_offsets() {
    // B-type immediates must be even (bit 0 is always 0).
    for &val in &[-4096i32, -256, -8, 0, 8, 128, 4094] {
        let inst = b_type(i_op::OP_BRANCH, 0, 0, 0, val);
        let d = decode(inst);
        assert_eq!(d.imm, val as i64, "B-type round-trip failed for imm={val}");
    }
}

#[test]
fn j_type_imm_round_trip_boundaries() {
    for &val in &[-1048576i32, -20, 0, 100, 1048574] {
        let inst = j_type(i_op::OP_JAL, 0, val);
        let d = decode(inst);
        assert_eq!(d.imm, val as i64, "J-type round-trip failed for imm={val}");
    }
}

#[test]
fn u_type_imm_round_trip() {
    for &imm20 in &[0u32, 1, 0x7FFFF, 0x80000, 0xFFFFF] {
        let inst = u_type(i_op::OP_LUI, 0, imm20);
        let d = decode(inst);
        let expected = (imm20 << 12) as i32 as i64;
        assert_eq!(
            d.imm, expected,
            "U-type round-trip failed for imm20={imm20:#x}"
        );
    }
}

// ══════════════════════════════════════════════════════════
// 19. NOP encoding
// ══════════════════════════════════════════════════════════

#[test]
fn decode_nop() {
    // NOP = ADDI x0, x0, 0
    let inst = i_type(i_op::OP_IMM, 0, i_f3::ADD_SUB, 0, 0);
    let d = decode(inst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.rd, 0);
    assert_eq!(d.rs1, 0);
    assert_eq!(d.imm, 0);
}
