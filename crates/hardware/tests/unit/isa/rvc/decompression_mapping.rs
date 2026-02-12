//! Compressed Instruction (RVC) Decompression Mapping Tests.
//!
//! Verifies that every compressed instruction expands to the correct
//! 32-bit equivalent. Tests cover all three quadrants (Q0, Q1, Q2)
//! and check register mappings, immediate extraction, and edge cases.

use riscv_core::isa::decode::decode;
use riscv_core::isa::rvc::expand::expand;

use riscv_core::isa::privileged::opcodes as sys_op;
use riscv_core::isa::rv64f::opcodes as f_op;
use riscv_core::isa::rv64i::funct3 as i_f3;
use riscv_core::isa::rv64i::funct7 as i_f7;
use riscv_core::isa::rv64i::opcodes as i_op;

// ──────────────────────────────────────────────────────────
// Helper: decode expanded instruction for field checks
// ──────────────────────────────────────────────────────────

/// Expand a 16-bit compressed instruction and decode the resulting 32-bit instruction.
fn expand_and_decode(cinst: u16) -> riscv_core::isa::instruction::Decoded {
    let expanded = expand(cinst);
    assert_ne!(
        expanded, 0,
        "Expansion must not produce illegal instruction 0 for {cinst:#06x}"
    );
    decode(expanded)
}

// ══════════════════════════════════════════════════════════
// Quadrant 0 (bits 1:0 = 00)
// ══════════════════════════════════════════════════════════

#[test]
fn rvc_c_addi4spn() {
    // C.ADDI4SPN rd', nzuimm → ADDI rd'+8, x2, nzuimm
    // Encoding: funct3=000 | imm[5:4|9:6|2|3] | rd' | 00
    // Let's encode nzuimm=16 (smallest non-zero aligned), rd'=0 → x8
    // imm bits: [9:6]=0, [5:4]=01, [3]=0, [2]=0 → nzuimm=16
    // inst[12:5] encode the immediate, inst[4:2] encode rd'
    // bit layout: [12:11]=imm[5:4], [10:7]=imm[9:6], [6]=imm[2], [5]=imm[3], [4:2]=rd', [1:0]=00
    let cinst: u16 = 0b000_01000000_000_00; // nzuimm=16, rd'=0(x8)
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.rs1, 2, "C.ADDI4SPN base must be x2 (sp)");
    assert_eq!(d.rd, 8, "rd' = 0 maps to x8");
    assert_eq!(d.imm, 16);
}

#[test]
fn rvc_c_addi4spn_zero_is_illegal() {
    // nzuimm=0 is reserved (illegal)
    let cinst: u16 = 0b000_00000000_000_00;
    let expanded = expand(cinst);
    assert_eq!(
        expanded, 0,
        "C.ADDI4SPN with nzuimm=0 must expand to illegal"
    );
}

#[test]
fn rvc_c_lw() {
    // C.LW rd', offset(rs1') → LW rd'+8, offset(rs1'+8)
    // funct3=010, offset 5-bit scaled by 4
    // Encode: rs1'=0(x8), rd'=1(x9), offset=0
    // bits: [15:13]=010 [12:10]=imm [9:7]=rs1' [6:5]=imm [4:2]=rd' [1:0]=00
    let cinst: u16 = 0b010_000_000_00_001_00; // rs1'=0(x8), rd'=1(x9), offset=0
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_LOAD);
    assert_eq!(d.funct3, i_f3::LW);
    assert_eq!(d.rs1, 8);
    assert_eq!(d.rd, 9);
}

#[test]
fn rvc_c_ld() {
    // C.LD rd', offset(rs1') → LD rd'+8, offset(rs1'+8)
    // funct3=011
    let cinst: u16 = 0b011_000_000_01_000_00;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_LOAD);
    assert_eq!(d.funct3, i_f3::LD);
}

#[test]
fn rvc_c_fld() {
    // C.FLD rd', offset(rs1') → FLD rd'+8, offset(rs1'+8)
    // funct3=001
    let cinst: u16 = 0b001_000_000_01_000_00;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, f_op::OP_LOAD_FP);
    assert_eq!(d.funct3, i_f3::LD);
}

#[test]
fn rvc_c_sw() {
    // C.SW rs2', offset(rs1') → SW rs2'+8, offset(rs1'+8)
    // funct3=110
    let cinst: u16 = 0b110_000_000_01_000_00;
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_STORE);
    assert_eq!(d.funct3, i_f3::SW);
}

#[test]
fn rvc_c_sd() {
    // C.SD rs2', offset(rs1') → SD rs2'+8, offset(rs1'+8)
    // funct3=111
    let cinst: u16 = 0b111_000_000_01_000_00;
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_STORE);
    assert_eq!(d.funct3, i_f3::SD);
}

#[test]
fn rvc_c_fsd() {
    // C.FSD rs2', offset(rs1') → FSD rs2'+8, offset(rs1'+8)
    // funct3=101
    let cinst: u16 = 0b101_000_000_01_000_00;
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, f_op::OP_STORE_FP);
}

// ══════════════════════════════════════════════════════════
// Quadrant 1 (bits 1:0 = 01)
// ══════════════════════════════════════════════════════════

#[test]
fn rvc_c_addi() {
    // C.ADDI rd, nzimm → ADDI rd, rd, nzimm
    // funct3=000, rd=1(x1), nzimm=1
    // [12]=0, [11:7]=00001(rd=1), [6:2]=00001(imm[4:0]=1), [1:0]=01
    let cinst: u16 = 0b000_0_00001_00001_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.rd, 1);
    assert_eq!(d.rs1, 1);
    assert_eq!(d.imm, 1);
}

#[test]
fn rvc_c_addi_negative() {
    // C.ADDI x1, -1: [12]=1, rd=1, imm[4:0]=11111
    let cinst: u16 = 0b000_1_00001_11111_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.imm, -1);
}

#[test]
fn rvc_c_addiw() {
    // C.ADDIW rd, imm → ADDIW rd, rd, imm
    // funct3=001, rd=5, imm=3
    let cinst: u16 = 0b001_0_00101_00011_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM_32);
    assert_eq!(d.rd, 5);
    assert_eq!(d.rs1, 5);
}

#[test]
fn rvc_c_addiw_rd0_illegal() {
    // C.ADDIW with rd=0 is reserved
    let cinst: u16 = 0b001_0_00000_00011_01;
    let expanded = expand(cinst);
    assert_eq!(expanded, 0);
}

#[test]
fn rvc_c_li() {
    // C.LI rd, imm → ADDI rd, x0, imm
    // funct3=010, rd=3, imm=7
    let cinst: u16 = 0b010_0_00011_00111_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.rd, 3);
    assert_eq!(d.rs1, 0, "C.LI uses x0 as source");
    assert_eq!(d.imm, 7);
}

#[test]
fn rvc_c_addi16sp() {
    // C.ADDI16SP nzimm → ADDI x2, x2, nzimm (rd=2)
    // funct3=011, rd=2
    // nzimm encoding: bit12=nzimm[9], bit6=nzimm[4], bit5=nzimm[6],
    //                 bits[4:3]=nzimm[8:7], bit2=nzimm[5]
    // For nzimm=16: nzimm[4]=1 → bit6=1, all others 0
    let cinst: u16 = 0b011_0_00010_10000_01; // bit6=1 → nzimm[4]=1 → nzimm=16
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.rd, 2);
    assert_eq!(d.rs1, 2);
    assert_eq!(d.imm, 16);
}

#[test]
fn rvc_c_lui() {
    // C.LUI rd, nzimm → LUI rd, nzimm (rd != 0, 2)
    // funct3=011, rd=3, nzimm[5]=0, nzimm[4:0]=00001 → nzimm=1
    let cinst: u16 = 0b011_0_00011_00001_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_LUI);
    assert_eq!(d.rd, 3);
}

#[test]
fn rvc_c_srli() {
    // C.SRLI rd', shamt → SRLI rd'+8, rd'+8, shamt
    // funct3=100, funct2=00, rd'=0(x8), shamt=1
    let cinst: u16 = 0b100_0_00_000_00001_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.funct3, i_f3::SRL_SRA);
    assert_eq!(d.rd, 8);
}

#[test]
fn rvc_c_srai() {
    // C.SRAI rd', shamt → SRAI rd'+8, rd'+8, shamt
    // funct3=100, funct2=01
    let cinst: u16 = 0b100_0_01_000_00001_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.funct3, i_f3::SRL_SRA);
    assert_eq!(d.funct7, i_f7::SRA);
}

#[test]
fn rvc_c_andi() {
    // C.ANDI rd', imm → ANDI rd'+8, rd'+8, imm
    // funct3=100, funct2=10
    let cinst: u16 = 0b100_0_10_000_00011_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.funct3, i_f3::AND);
    assert_eq!(d.rd, 8);
}

#[test]
fn rvc_c_sub() {
    // C.SUB rd', rs2' → SUB rd'+8, rd'+8, rs2'+8
    // funct3=100, funct2=11, bit12=0, sub_op=00
    let cinst: u16 = 0b100_0_11_000_00_001_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.funct7, i_f7::SUB);
    assert_eq!(d.rd, 8);
    assert_eq!(d.rs1, 8);
    assert_eq!(d.rs2, 9);
}

#[test]
fn rvc_c_xor() {
    // C.XOR: funct3=100, funct2=11, bit12=0, sub_op=01
    let cinst: u16 = 0b100_0_11_000_01_001_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::XOR);
}

#[test]
fn rvc_c_or() {
    // C.OR: funct3=100, funct2=11, bit12=0, sub_op=10
    let cinst: u16 = 0b100_0_11_000_10_001_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::OR);
}

#[test]
fn rvc_c_and() {
    // C.AND: funct3=100, funct2=11, bit12=0, sub_op=11
    let cinst: u16 = 0b100_0_11_000_11_001_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::AND);
}

#[test]
fn rvc_c_subw() {
    // C.SUBW: funct3=100, funct2=11, bit12=1, sub_op=00
    let cinst: u16 = 0b100_1_11_000_00_001_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.funct7, i_f7::SUB);
}

#[test]
fn rvc_c_addw() {
    // C.ADDW: funct3=100, funct2=11, bit12=1, sub_op=01
    let cinst: u16 = 0b100_1_11_000_01_001_01;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG_32);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.funct7, i_f7::DEFAULT, "ADDW uses funct7=0");
}

#[test]
fn rvc_c_j() {
    // C.J offset → JAL x0, offset
    // funct3=101 (bits[15:13])
    // offset bits: bits[5:3]→offset[3:1], bit[11]→offset[4], bit[2]→offset[5],
    //              bit[7]→offset[6], bit[6]→offset[7], bits[10:9]→offset[9:8],
    //              bit[8]→offset[10], bit[12]→offset[11]
    // For offset=2: need offset[1]=1 → bit3=1, all other offset bits=0
    let cinst: u16 = 0xA009; // 0b1010_0000_0000_1001: funct3=101, bit3=1, op=01
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_JAL);
    assert_eq!(d.rd, 0, "C.J links to x0");
}

#[test]
fn rvc_c_beqz() {
    // C.BEQZ rs1', offset → BEQ rs1'+8, x0, offset
    // funct3=110
    let cinst: u16 = 0b110_0_00_000_00_001_01; // rs1'=0(x8), small offset
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_BRANCH);
    assert_eq!(d.funct3, i_f3::BEQ);
    assert_eq!(d.rs1, 8);
    assert_eq!(d.rs2, 0);
}

#[test]
fn rvc_c_bnez() {
    // C.BNEZ rs1', offset → BNE rs1'+8, x0, offset
    // funct3=111
    let cinst: u16 = 0b111_0_00_000_00_001_01;
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_BRANCH);
    assert_eq!(d.funct3, i_f3::BNE);
    assert_eq!(d.rs1, 8);
}

// ══════════════════════════════════════════════════════════
// Quadrant 2 (bits 1:0 = 10)
// ══════════════════════════════════════════════════════════

#[test]
fn rvc_c_slli() {
    // C.SLLI rd, shamt → SLLI rd, rd, shamt
    // funct3=000, rd=1, shamt=4
    let cinst: u16 = 0b000_0_00001_00100_10;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_IMM);
    assert_eq!(d.funct3, i_f3::SLL);
    assert_eq!(d.rd, 1);
    assert_eq!(d.rs1, 1);
}

#[test]
fn rvc_c_slli_rd0_illegal() {
    // C.SLLI with rd=0 is reserved
    let cinst: u16 = 0b000_0_00000_00100_10;
    let expanded = expand(cinst);
    assert_eq!(expanded, 0);
}

#[test]
fn rvc_c_lwsp() {
    // C.LWSP rd, offset(sp) → LW rd, offset(x2)
    // funct3=010, rd=1
    // offset[5]=bit12, offset[4:2]=bits[6:4], offset[7:6]=bits[3:2]
    let cinst: u16 = 0b010_0_00001_00000_10; // rd=1, offset=0
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_LOAD);
    assert_eq!(d.funct3, i_f3::LW);
    assert_eq!(d.rd, 1);
    assert_eq!(d.rs1, 2, "C.LWSP base is x2 (sp)");
}

#[test]
fn rvc_c_lwsp_rd0_illegal() {
    let cinst: u16 = 0b010_0_00000_00000_10;
    let expanded = expand(cinst);
    assert_eq!(expanded, 0, "C.LWSP with rd=0 is reserved");
}

#[test]
fn rvc_c_ldsp() {
    // C.LDSP rd, offset(sp) → LD rd, offset(x2)
    // funct3=011
    let cinst: u16 = 0b011_0_00001_00000_10;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_LOAD);
    assert_eq!(d.funct3, i_f3::LD);
    assert_eq!(d.rs1, 2);
}

#[test]
fn rvc_c_ldsp_rd0_illegal() {
    let cinst: u16 = 0b011_0_00000_00000_10;
    let expanded = expand(cinst);
    assert_eq!(expanded, 0, "C.LDSP with rd=0 is reserved");
}

#[test]
fn rvc_c_fldsp() {
    // C.FLDSP rd, offset(sp) → FLD rd, offset(x2)
    // funct3=001
    let cinst: u16 = 0b001_0_00001_00000_10;
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, f_op::OP_LOAD_FP);
    assert_eq!(d.funct3, i_f3::LD);
    assert_eq!(d.rs1, 2);
}

#[test]
fn rvc_c_jr() {
    // C.JR rs1 → JALR x0, rs1, 0 (bit12=0, rs2=0, rs1!=0)
    // funct3=100
    let cinst: u16 = 0b100_0_00101_00000_10; // rs1=5, rs2=0, bit12=0
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_JALR);
    assert_eq!(d.rd, 0, "C.JR links to x0");
    assert_eq!(d.rs1, 5);
}

#[test]
fn rvc_c_mv() {
    // C.MV rd, rs2 → ADD rd, x0, rs2 (bit12=0, rs2!=0)
    // funct3=100
    let cinst: u16 = 0b100_0_00011_00101_10; // rd=3, rs2=5, bit12=0
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.rd, 3);
    assert_eq!(d.rs1, 0, "C.MV uses x0 as rs1");
    assert_eq!(d.rs2, 5);
}

#[test]
fn rvc_c_ebreak() {
    // C.EBREAK → EBREAK (bit12=1, rs1=0, rs2=0)
    let cinst: u16 = 0b100_1_00000_00000_10;
    let expanded = expand(cinst);
    assert_eq!(expanded, sys_op::EBREAK);
}

#[test]
fn rvc_c_jalr() {
    // C.JALR rs1 → JALR x1, rs1, 0 (bit12=1, rs2=0, rs1!=0)
    let cinst: u16 = 0b100_1_00101_00000_10; // rs1=5
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_JALR);
    assert_eq!(d.rd, 1, "C.JALR links to x1 (ra)");
    assert_eq!(d.rs1, 5);
}

#[test]
fn rvc_c_add() {
    // C.ADD rd, rs2 → ADD rd, rd, rs2 (bit12=1, rs2!=0)
    let cinst: u16 = 0b100_1_00011_00101_10; // rd=3, rs2=5
    let d = expand_and_decode(cinst);
    assert_eq!(d.opcode, i_op::OP_REG);
    assert_eq!(d.funct3, i_f3::ADD_SUB);
    assert_eq!(d.funct7, i_f7::DEFAULT);
    assert_eq!(d.rd, 3);
    assert_eq!(d.rs1, 3, "C.ADD uses rd as rs1");
    assert_eq!(d.rs2, 5);
}

#[test]
fn rvc_c_swsp() {
    // C.SWSP rs2, offset(sp) → SW rs2, offset(x2)
    // funct3=110
    let cinst: u16 = 0b110_000000_00011_10; // rs2=3, offset=0
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_STORE);
    assert_eq!(d.funct3, i_f3::SW);
    assert_eq!(d.rs1, 2, "C.SWSP base is x2 (sp)");
    assert_eq!(d.rs2, 3);
}

#[test]
fn rvc_c_sdsp() {
    // C.SDSP rs2, offset(sp) → SD rs2, offset(x2)
    // funct3=111
    let cinst: u16 = 0b111_000000_00011_10; // rs2=3, offset=0
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, i_op::OP_STORE);
    assert_eq!(d.funct3, i_f3::SD);
    assert_eq!(d.rs1, 2);
}

#[test]
fn rvc_c_fsdsp() {
    // C.FSDSP rs2, offset(sp) → FSD rs2, offset(x2)
    // funct3=101
    let cinst: u16 = 0b101_000000_00011_10; // rs2=3, offset=0
    let expanded = expand(cinst);
    assert_ne!(expanded, 0);
    let d = decode(expanded);
    assert_eq!(d.opcode, f_op::OP_STORE_FP);
    assert_eq!(d.rs1, 2);
}

// ══════════════════════════════════════════════════════════
// Edge cases
// ══════════════════════════════════════════════════════════

#[test]
fn rvc_quadrant_3_is_not_compressed() {
    // bits[1:0] = 11 means 32-bit instruction, not compressed
    let cinst: u16 = 0x0003; // opcode = 0b11
    let expanded = expand(cinst);
    assert_eq!(
        expanded, 0,
        "Quadrant 3 (32-bit) should not be handled by RVC expander"
    );
}

#[test]
fn rvc_all_register_mappings_q0() {
    // Verify compressed register rd'=0..7 maps to x8..x15
    for rd_prime in 0u16..8 {
        // C.LW rd', 0(x8) - funct3=010, rs1'=0(x8), offset=0
        let cinst: u16 = 0b010_000_000_00_000_00 | (rd_prime << 2);
        let d = expand_and_decode(cinst);
        assert_eq!(
            d.rd,
            (8 + rd_prime) as usize,
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
        let cinst: u16 = 0b010_000_000_00_000_00 | (rs1_prime << 7);
        let d = expand_and_decode(cinst);
        assert_eq!(
            d.rs1,
            (8 + rs1_prime) as usize,
            "rs1'={rs1_prime} should map to x{}",
            8 + rs1_prime
        );
    }
}
