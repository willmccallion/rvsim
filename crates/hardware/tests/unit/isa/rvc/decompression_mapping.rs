//! Compressed Instruction (RVC) Decompression Mapping Tests.
//!
//! Verifies that every compressed instruction expands to the correct
//! 32-bit equivalent. Tests cover all three quadrants (Q0, Q1, Q2)
//! and check register mappings, immediate extraction, and edge cases.

use rvsim_core::isa::instruction::decode;
use rvsim_core::isa::reg::RegIdx;
use rvsim_core::isa::rvc::expand;

use rvsim_core::isa::encoding::privileged as sys_op;
use rvsim_core::isa::encoding::rv64f::opcodes as f_op;
use rvsim_core::isa::encoding::rv64i::funct3 as i_f3;
use rvsim_core::isa::encoding::rv64i::funct7 as i_f7;
use rvsim_core::isa::encoding::rv64i::opcodes as i_op;

/// Expand a 16-bit compressed instruction and decode the resulting 32-bit instruction.
fn expand_and_decode(cinst: u16) -> rvsim_core::isa::instruction::Decoded {
    let expanded = expand(cinst);
    assert_ne!(expanded, 0, "Expansion must not produce illegal instruction 0 for {cinst:#06x}");
    decode(expanded)
}

#[test]
fn rvc_c_addi4spn() {
    // C.ADDI4SPN rd', nzuimm → ADDI rd'+8, x2, nzuimm
    let cinst: u16 = 0b0000_1000_0000_0000; // nzuimm=16, rd'=0(x8)
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.rs1, RegIdx::new(2), "C.ADDI4SPN base must be x2 (sp)");
    assert_eq!(d.rd, RegIdx::new(8), "rd' = 0 maps to x8");
    assert_eq!(d.imm, 16);
}

#[test]
fn rvc_c_addi4spn_zero_is_illegal() {
    // nzuimm=0 is reserved (illegal)
    let cinst: u16 = 0b0000_0000_0000_0000;
    let expanded = expand(cinst);
    assert_eq!(expanded, 0, "C.ADDI4SPN with nzuimm=0 must expand to illegal");
}

#[test]
fn rvc_c_lw() {
    // C.LW rd', offset(rs1') → LW rd'+8, offset(rs1'+8)
    let cinst: u16 = 0b0100_0000_0000_0100; // rs1'=0(x8), rd'=1(x9), offset=0
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_LOAD);
    assert_eq!(d.funct3, i_f3::LW);
    assert_eq!(d.rs1, RegIdx::new(8));
    assert_eq!(d.rd, RegIdx::new(9));
}

#[test]
fn rvc_c_ld() {
    // C.LD rd', offset(rs1') → LD rd'+8, offset(rs1'+8)
    let cinst: u16 = 0b0110_0000_0010_0000;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_LOAD);
    assert_eq!(d.funct3, i_f3::LD);
}

#[test]
fn rvc_c_fld() {
    // C.FLD rd', offset(rs1') → FLD rd'+8, offset(rs1'+8)
    let cinst: u16 = 0b0010_0000_0010_0000;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, f_op::OP_LOAD_FP);
    assert_eq!(d.funct3, i_f3::LD);
}

#[test]
fn rvc_c_sw() {
    // C.SW rs2', offset(rs1') → SW rs2'+8, offset(rs1'+8)
    let cinst: u16 = 0b1100_0000_0010_0000;
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_STORE);
    assert_eq!(d.funct3, i_f3::SW);
}

#[test]
fn rvc_c_sd() {
    // C.SD rs2', offset(rs1') → SD rs2'+8, offset(rs1'+8)
    let cinst: u16 = 0b1110_0000_0010_0000;
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_STORE);
    assert_eq!(d.funct3, i_f3::SD);
}

#[test]
fn rvc_c_fsd() {
    // C.FSD rs2', offset(rs1') → FSD rs2'+8, offset(rs1'+8)
    let cinst: u16 = 0b1010_0000_0010_0000;
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, f_op::OP_STORE_FP);
}

#[test]
fn rvc_c_addi() {
    // C.ADDI rd, nzimm → ADDI rd, rd, nzimm. Encoded: rd=1(x1), nzimm=1.
    let cinst: u16 = 0b0000_0000_1000_0101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.rd, RegIdx::new(1));
    assert_eq!(d.rs1, RegIdx::new(1));
    assert_eq!(d.imm, 1);
}

#[test]
fn rvc_c_addi_negative() {
    // C.ADDI x1, -1: [12]=1, rd=1, imm[4:0]=11111
    let cinst: u16 = 0b0001_0000_1111_1101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.imm, -1);
}

#[test]
fn rvc_c_addiw() {
    // C.ADDIW rd, imm → ADDIW rd, rd, imm
    let cinst: u16 = 0b0010_0010_1000_1101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM_32);
    assert_eq!(d.rd, RegIdx::new(5));
    assert_eq!(d.rs1, RegIdx::new(5));
}

#[test]
fn rvc_c_addiw_rd0_illegal() {
    // C.ADDIW with rd=0 is reserved
    let cinst: u16 = 0b0010_0000_0000_1101;
    let expanded = expand(cinst);
    assert_eq!(expanded, 0);
}

#[test]
fn rvc_c_li() {
    // C.LI rd, imm → ADDI rd, x0, imm
    let cinst: u16 = 0b0100_0001_1001_1101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.rd, RegIdx::new(3));
    assert_eq!(d.rs1, RegIdx::new(0), "C.LI uses x0 as source");
    assert_eq!(d.imm, 7);
}

#[test]
fn rvc_c_addi16sp() {
    // C.ADDI16SP nzimm → ADDI x2, x2, nzimm (rd=2). Encoded: nzimm=16.
    let cinst: u16 = 0b0110_0001_0100_0001; // bit6=1 → nzimm[4]=1 → nzimm=16
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.rd, RegIdx::new(2));
    assert_eq!(d.rs1, RegIdx::new(2));
    assert_eq!(d.imm, 16);
}

#[test]
fn rvc_c_lui() {
    // C.LUI rd, nzimm → LUI rd, nzimm (rd != 0, 2)
    let cinst: u16 = 0b0110_0001_1000_0101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_LUI);
    assert_eq!(d.rd, RegIdx::new(3));
}

#[test]
fn rvc_c_srli() {
    // C.SRLI rd', shamt → SRLI rd'+8, rd'+8, shamt
    let cinst: u16 = 0b1000_0000_0000_0101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.funct3, i_f3::SRL_SRA);
    assert_eq!(d.rd, RegIdx::new(8));
}

#[test]
fn rvc_c_srai() {
    // C.SRAI rd', shamt → SRAI rd'+8, rd'+8, shamt
    let cinst: u16 = 0b1000_0100_0000_0101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.funct3, i_f3::SRL_SRA);
    assert_eq!(d.funct7, i_f7::SRA);
}

#[test]
fn rvc_c_andi() {
    // C.ANDI rd', imm → ANDI rd'+8, rd'+8, imm
    let cinst: u16 = 0b1000_1000_0000_1101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.funct3, i_f3::AND);
    assert_eq!(d.rd, RegIdx::new(8));
}

#[test]
fn rvc_c_sub() {
    // C.SUB rd', rs2' → SUB rd'+8, rd'+8, rs2'+8
    let cinst: u16 = 0b1000_1100_0000_0101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.funct7, i_f7::SUB);
    assert_eq!(d.rd, RegIdx::new(8));
    assert_eq!(d.rs1, RegIdx::new(8));
    assert_eq!(d.rs2, RegIdx::new(9));
}

#[test]
fn rvc_c_xor() {
    // C.XOR: funct3=100, funct2=11, bit12=0, sub_op=01
    let cinst: u16 = 0b1000_1100_0010_0101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::XOR);
}

#[test]
fn rvc_c_or() {
    // C.OR: funct3=100, funct2=11, bit12=0, sub_op=10
    let cinst: u16 = 0b1000_1100_0100_0101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::OR);
}

#[test]
fn rvc_c_and() {
    // C.AND: funct3=100, funct2=11, bit12=0, sub_op=11
    let cinst: u16 = 0b1000_1100_0110_0101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::AND);
}

#[test]
fn rvc_c_subw() {
    // C.SUBW: funct3=100, funct2=11, bit12=1, sub_op=00
    let cinst: u16 = 0b1001_1100_0000_0101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.funct7, i_f7::SUB);
}

#[test]
fn rvc_c_addw() {
    // C.ADDW: funct3=100, funct2=11, bit12=1, sub_op=01
    let cinst: u16 = 0b1001_1100_0010_0101;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.funct7, i_f7::DEFAULT, "ADDW uses funct7=0");
}

#[test]
fn rvc_c_j() {
    // C.J offset → JAL x0, offset.
    let cinst: u16 = 0xA009; // 0b1010_0000_0000_1001: funct3=101, bit3=1, op=01
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_JAL);
    assert_eq!(d.rd, RegIdx::new(0), "C.J links to x0");
}

#[test]
fn rvc_c_beqz() {
    // C.BEQZ rs1', offset → BEQ rs1'+8, x0, offset
    let cinst: u16 = 0b1100_0000_0000_0101; // rs1'=0(x8), small offset
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_BRANCH);
    assert_eq!(d.funct3, i_f3::BEQ);
    assert_eq!(d.rs1, RegIdx::new(8));
    assert_eq!(d.rs2, RegIdx::new(0));
}

#[test]
fn rvc_c_bnez() {
    // C.BNEZ rs1', offset → BNE rs1'+8, x0, offset
    let cinst: u16 = 0b1110_0000_0000_0101;
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_BRANCH);
    assert_eq!(d.funct3, i_f3::BNE);
    assert_eq!(d.rs1, RegIdx::new(8));
}

#[test]
fn rvc_c_slli() {
    // C.SLLI rd, shamt → SLLI rd, rd, shamt
    let cinst: u16 = 0b0000_0000_1001_0010;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.funct3, i_f3::SLL);
    assert_eq!(d.rd, RegIdx::new(1));
    assert_eq!(d.rs1, RegIdx::new(1));
}

#[test]
fn rvc_c_slli_rd0_illegal() {
    // C.SLLI with rd=0 is reserved
    let cinst: u16 = 0b0000_0000_0001_0010;
    let expanded = expand(cinst);
    assert_eq!(expanded, 0);
}

#[test]
fn rvc_c_lwsp() {
    // C.LWSP rd, offset(sp) → LW rd, offset(x2).
    let cinst: u16 = 0b0100_0000_1000_0010; // rd=1, offset=0
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_LOAD);
    assert_eq!(d.funct3, i_f3::LW);
    assert_eq!(d.rd, RegIdx::new(1));
    assert_eq!(d.rs1, RegIdx::new(2), "C.LWSP base is x2 (sp)");
}

#[test]
fn rvc_c_lwsp_rd0_illegal() {
    let cinst: u16 = 0b0100_0000_0000_0010;
    let expanded = expand(cinst);
    assert_eq!(expanded, 0, "C.LWSP with rd=0 is reserved");
}

#[test]
fn rvc_c_ldsp() {
    // C.LDSP rd, offset(sp) → LD rd, offset(x2)
    let cinst: u16 = 0b0110_0000_1000_0010;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_LOAD);
    assert_eq!(d.funct3, i_f3::LD);
    assert_eq!(d.rs1, RegIdx::new(2));
}

#[test]
fn rvc_c_ldsp_rd0_illegal() {
    let cinst: u16 = 0b0110_0000_0000_0010;
    let expanded = expand(cinst);
    assert_eq!(expanded, 0, "C.LDSP with rd=0 is reserved");
}

#[test]
fn rvc_c_fldsp() {
    // C.FLDSP rd, offset(sp) → FLD rd, offset(x2)
    let cinst: u16 = 0b0010_0000_1000_0010;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, f_op::OP_LOAD_FP);
    assert_eq!(d.funct3, i_f3::LD);
    assert_eq!(d.rs1, RegIdx::new(2));
}

#[test]
fn rvc_c_jr() {
    // C.JR rs1 → JALR x0, rs1, 0 (bit12=0, rs2=0, rs1!=0)
    let cinst: u16 = 0b1000_0010_1000_0010; // rs1=5, rs2=0, bit12=0
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_JALR);
    assert_eq!(d.rd, RegIdx::new(0), "C.JR links to x0");
    assert_eq!(d.rs1, RegIdx::new(5));
}

#[test]
fn rvc_c_mv() {
    // C.MV rd, rs2 → ADD rd, x0, rs2 (bit12=0, rs2!=0)
    let cinst: u16 = 0b1000_0001_1001_0110; // rd=3, rs2=5, bit12=0
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.rd, RegIdx::new(3));
    assert_eq!(d.rs1, RegIdx::new(0), "C.MV uses x0 as rs1");
    assert_eq!(d.rs2, RegIdx::new(5));
}

#[test]
fn rvc_c_ebreak() {
    // C.EBREAK → EBREAK (bit12=1, rs1=0, rs2=0)
    let cinst: u16 = 0b1001_0000_0000_0010;
    let expanded = expand(cinst);
    assert_eq!(expanded, sys_op::EBREAK);
}

#[test]
fn rvc_c_jalr() {
    // C.JALR rs1 → JALR x1, rs1, 0 (bit12=1, rs2=0, rs1!=0)
    let cinst: u16 = 0b1001_0010_1000_0010; // rs1=5
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_JALR);
    assert_eq!(d.rd, RegIdx::new(1), "C.JALR links to x1 (ra)");
    assert_eq!(d.rs1, RegIdx::new(5));
}

#[test]
fn rvc_c_add() {
    // C.ADD rd, rs2 → ADD rd, rd, rs2 (bit12=1, rs2!=0)
    let cinst: u16 = 0b1001_0001_1001_0110; // rd=3, rs2=5
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.funct7, i_f7::DEFAULT);
    assert_eq!(d.rd, RegIdx::new(3));
    assert_eq!(d.rs1, RegIdx::new(3), "C.ADD uses rd as rs1");
    assert_eq!(d.rs2, RegIdx::new(5));
}

#[test]
fn rvc_c_swsp() {
    // C.SWSP rs2, offset(sp) → SW rs2, offset(x2)
    let cinst: u16 = 0b1100_0000_0000_1110; // rs2=3, offset=0
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_STORE);
    assert_eq!(d.funct3, i_f3::SW);
    assert_eq!(d.rs1, RegIdx::new(2), "C.SWSP base is x2 (sp)");
    assert_eq!(d.rs2, RegIdx::new(3));
}

#[test]
fn rvc_c_sdsp() {
    // C.SDSP rs2, offset(sp) → SD rs2, offset(x2)
    let cinst: u16 = 0b1110_0000_0000_1110; // rs2=3, offset=0
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_STORE);
    assert_eq!(d.funct3, i_f3::SD);
    assert_eq!(d.rs1, RegIdx::new(2));
}

#[test]
fn rvc_c_fsdsp() {
    // C.FSDSP rs2, offset(sp) → FSD rs2, offset(x2)
    let cinst: u16 = 0b1010_0000_0000_1110; // rs2=3, offset=0
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, f_op::OP_STORE_FP);
    assert_eq!(d.rs1, RegIdx::new(2));
}

#[test]
fn rvc_quadrant_3_is_not_compressed() {
    // bits[1:0] = 11 means 32-bit instruction, not compressed
    let cinst: u16 = 0x0003; // opcode = 0b11
    let expanded = expand(cinst);
    assert_eq!(expanded, 0, "Quadrant 3 (32-bit) should not be handled by RVC expander");
}

#[test]
fn rvc_all_register_mappings_q0() {
    // Verify compressed register rd'=0..7 maps to x8..x15
    for rd_prime in 0u16..8 {
        // C.LW rd', 0(x8) - funct3=010, rs1'=0(x8), offset=0
        let cinst: u16 = 0b0100_0000_0000_0000 | (rd_prime << 2);
        let d = expand_and_decode(cinst);
        assert_eq!(
            d.rd,
            RegIdx::new((8 + rd_prime) as u8),
            "rd'={rd_prime} should map to x{}",
            8 + rd_prime
        );
    }
}

#[test]
fn rvc_all_register_mappings_q0_rs1() {
    // Verify compressed register rs1'=0..7 maps to x8..x15
    for rs1_prime in 0u16..8 {
        // C.LW x8, 0(rs1') - rd'=0, varying rs1'
        let cinst: u16 = 0b0100_0000_0000_0000 | (rs1_prime << 7);
        let d = expand_and_decode(cinst);
        assert_eq!(
            d.rs1,
            RegIdx::new((8 + rs1_prime) as u8),
            "rs1'={rs1_prime} should map to x{}",
            8 + rs1_prime
        );
    }
}
