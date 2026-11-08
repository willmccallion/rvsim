//! `decode()` gives back every field of every instruction format, with the
//! immediate sign-extended as its format places it.

use proptest::prelude::*;
use proptest::sample::select;

use crate::isa::csr::CsrAddr;
use crate::isa::encode::{BType, IType, JType, R4Type, RType, SType, UType};
use crate::isa::encoding::privileged::OP_SYSTEM;
use crate::isa::encoding::rv64a::opcodes::OP_AMO;
use crate::isa::encoding::rv64f::opcodes::{
    OP_FMADD, OP_FMSUB, OP_FNMADD, OP_FNMSUB, OP_FP, OP_LOAD_FP, OP_STORE_FP,
};
use crate::isa::encoding::rv64i::opcodes::{
    OP_AUIPC, OP_BRANCH, OP_IMM, OP_IMM_32, OP_JAL, OP_JALR, OP_LOAD, OP_LUI, OP_MISC_MEM, OP_REG,
    OP_REG_32, OP_STORE,
};
use crate::isa::instruction::{InstructionBits, decode, is_compressed};
use crate::isa::reg::RegIdx;
use crate::tests::support::seeded_cases;

static R_OPCODES: [u32; 4] = [OP_REG, OP_REG_32, OP_FP, OP_AMO];
static I_OPCODES: [u32; 6] = [OP_IMM, OP_IMM_32, OP_LOAD, OP_LOAD_FP, OP_JALR, OP_MISC_MEM];
static S_OPCODES: [u32; 2] = [OP_STORE, OP_STORE_FP];
static U_OPCODES: [u32; 2] = [OP_LUI, OP_AUIPC];
static R4_OPCODES: [u32; 4] = [OP_FMADD, OP_FMSUB, OP_FNMSUB, OP_FNMADD];

fn reg() -> impl Strategy<Value = RegIdx> {
    (0u8..32).prop_map(RegIdx::new)
}

fn twelve_bit_immediate() -> impl Strategy<Value = i32> {
    -2048i32..=2047
}

proptest! {
    #![proptest_config(seeded_cases(1024))]

    #[test]
    fn an_r_type_decodes_to_its_fields_and_no_immediate(
        opcode in select(&R_OPCODES[..]),
        (rd, rs1, rs2) in (reg(), reg(), reg()),
        funct3 in 0u32..8,
        funct7 in 0u32..128,
    ) {
        let decoded = decode(RType { opcode, funct3, funct7, rd, rs1, rs2 }.encode());

        prop_assert_eq!(
            (decoded.opcode, decoded.rd, decoded.rs1, decoded.rs2),
            (opcode, rd, rs1, rs2)
        );
        prop_assert_eq!((decoded.funct3, decoded.funct7, decoded.imm), (funct3, funct7, 0));
    }

    #[test]
    fn an_i_type_decodes_to_its_fields_and_sign_extended_immediate(
        opcode in select(&I_OPCODES[..]),
        (rd, rs1) in (reg(), reg()),
        funct3 in 0u32..8,
        imm in twelve_bit_immediate(),
    ) {
        let decoded = decode(IType { opcode, funct3, rd, rs1, imm }.encode());

        prop_assert_eq!(
            (decoded.opcode, decoded.rd, decoded.rs1, decoded.funct3, decoded.imm),
            (opcode, rd, rs1, funct3, i64::from(imm))
        );
    }

    #[test]
    fn a_system_instruction_carries_its_csr_and_no_immediate(
        (rd, rs1) in (reg(), reg()),
        funct3 in 0u32..8,
        imm in twelve_bit_immediate(),
    ) {
        let word = IType { opcode: OP_SYSTEM, funct3, rd, rs1, imm }.encode();

        prop_assert_eq!(word.csr(), CsrAddr::from_u32(imm.cast_unsigned() & 0xFFF));
        prop_assert_eq!(decode(word).imm, 0);
    }

    #[test]
    fn an_s_type_decodes_to_its_fields_and_split_immediate(
        opcode in select(&S_OPCODES[..]),
        (rs1, rs2) in (reg(), reg()),
        funct3 in 0u32..8,
        imm in twelve_bit_immediate(),
    ) {
        let decoded = decode(SType { opcode, funct3, rs1, rs2, imm }.encode());

        prop_assert_eq!(
            (decoded.opcode, decoded.rs1, decoded.rs2, decoded.funct3, decoded.imm),
            (opcode, rs1, rs2, funct3, i64::from(imm))
        );
    }

    #[test]
    fn a_branch_decodes_to_its_fields_and_even_offset(
        (rs1, rs2) in (reg(), reg()),
        funct3 in 0u32..8,
        offset in (-2048i32..2048).prop_map(|halves| halves * 2),
    ) {
        let decoded = decode(BType { opcode: OP_BRANCH, funct3, rs1, rs2, offset }.encode());

        prop_assert_eq!(
            (decoded.rs1, decoded.rs2, decoded.funct3, decoded.imm),
            (rs1, rs2, funct3, i64::from(offset))
        );
    }

    #[test]
    fn a_u_type_decodes_to_its_upper_immediate_sign_extended(
        opcode in select(&U_OPCODES[..]),
        rd in reg(),
        imm20 in 0u32..1 << 20,
    ) {
        let decoded = decode(UType { opcode, rd, imm20 }.encode());

        prop_assert_eq!(
            (decoded.opcode, decoded.rd, decoded.imm),
            (opcode, rd, i64::from((imm20 << 12).cast_signed()))
        );
    }

    #[test]
    fn a_jal_decodes_to_its_destination_and_even_offset(
        rd in reg(),
        offset in (-(1i32 << 19)..1 << 19).prop_map(|halves| halves * 2),
    ) {
        let decoded = decode(JType { opcode: OP_JAL, rd, offset }.encode());

        prop_assert_eq!((decoded.rd, decoded.imm), (rd, i64::from(offset)));
    }

    #[test]
    fn a_fused_multiply_add_carries_its_third_source(
        opcode in select(&R4_OPCODES[..]),
        (rd, rs1, rs2, rs3) in (reg(), reg(), reg(), reg()),
        funct3 in 0u32..8,
        fmt in 0u32..4,
    ) {
        let word = R4Type { opcode, funct3, fmt, rd, rs1, rs2, rs3 }.encode();
        let decoded = decode(word);

        prop_assert_eq!(word.rs3(), rs3);
        prop_assert_eq!(
            (decoded.opcode, decoded.rd, decoded.rs1, decoded.rs2, decoded.funct3, decoded.imm),
            (opcode, rd, rs1, rs2, funct3, 0)
        );
    }
}

#[test]
fn every_field_of_an_all_ones_word_is_all_ones() {
    let word = u32::MAX;

    assert_eq!((word.opcode(), word.funct3(), word.funct7()), (0x7F, 7, 0x7F));
    assert_eq!(
        (word.rd(), word.rs1(), word.rs2(), word.rs3()),
        (RegIdx::new(31), RegIdx::new(31), RegIdx::new(31), RegIdx::new(31))
    );
    assert_eq!(word.csr(), CsrAddr::from_u32(0xFFF));
}

#[test]
fn low_bits_other_than_0b11_mark_a_compressed_instruction() {
    const C_NOP: u16 = 0x0001;
    const C_LI_A0_0: u16 = 0x4501;
    const C_RET: u16 = 0x8082;
    const C_ILLEGAL: u16 = 0x0000;
    const ADDI_LOW_HALF: u16 = 0x0013;
    const ECALL_LOW_HALF: u16 = 0x0073;
    let compressed = [C_NOP, C_LI_A0_0, C_RET, C_ILLEGAL];
    let standard = [ADDI_LOW_HALF, ECALL_LOW_HALF];

    assert!(compressed.iter().all(|&h| is_compressed(h)));
    assert!(standard.iter().all(|&h| !is_compressed(h)));
}
