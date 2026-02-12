//! Decode Stage Unit Tests.
//!
//! Verifies that the decode stage correctly transforms IF/ID latch entries
//! into ID/EX latch entries with proper control signals, register reads,
//! immediate extraction, and hazard detection.
//!
//! Tests are organised into the following categories:
//!   1. NOP / zero-instruction handling
//!   2. Integer ALU control signals (R-type, I-type, OP-32 variants)
//!   3. Load / Store control signals and memory widths
//!   4. Branch and Jump control signals
//!   5. Upper-immediate instructions (LUI, AUIPC)
//!   6. Register file reads (source operands)
//!   7. Fetch-trap propagation
//!   8. Illegal instruction trap generation
//!   9. Intra-bundle hazard detection (superscalar)

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use riscv_core::core::pipeline::latches::IfIdEntry;
use riscv_core::core::pipeline::signals::{AluOp, MemWidth, OpASrc, OpBSrc};
use riscv_core::core::pipeline::stages::decode_stage;

// ══════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════

/// Build a minimal TestContext (no memory needed for decode).
fn ctx() -> TestContext {
    TestContext::new()
}

/// Place a single raw instruction in the IF/ID latch and run decode.
/// Returns the first IdExEntry produced.
fn decode_one(tc: &mut TestContext, inst: u32) -> riscv_core::core::pipeline::latches::IdExEntry {
    tc.cpu.if_id.entries = vec![IfIdEntry {
        pc: 0x8000_0000,
        inst,
        inst_size: 4,
        pred_taken: false,
        pred_target: 0,
        trap: None,
    }];
    decode_stage(&mut tc.cpu);
    assert!(
        !tc.cpu.id_ex.entries.is_empty(),
        "decode_stage should produce at least one entry for non-NOP instruction {:#010x}",
        inst
    );
    tc.cpu.id_ex.entries.remove(0)
}

/// Place a single raw instruction in IF/ID, run decode, and expect it to
/// be consumed but produce NO id_ex entry (i.e. it was treated as NOP).
fn decode_expect_nop(tc: &mut TestContext, inst: u32) {
    tc.cpu.if_id.entries = vec![IfIdEntry {
        pc: 0x8000_0000,
        inst,
        inst_size: 4,
        pred_taken: false,
        pred_target: 0,
        trap: None,
    }];
    decode_stage(&mut tc.cpu);
    assert!(
        tc.cpu.id_ex.entries.is_empty(),
        "Instruction {:#010x} should be treated as NOP (no id_ex entry)",
        inst
    );
}

// ══════════════════════════════════════════════════════════
// 1. NOP / Zero handling
// ══════════════════════════════════════════════════════════

#[test]
fn nop_is_consumed_silently() {
    let mut tc = ctx();
    let nop = InstructionBuilder::new().nop().build();
    decode_expect_nop(&mut tc, nop);
}

#[test]
fn zero_instruction_is_consumed_silently() {
    let mut tc = ctx();
    decode_expect_nop(&mut tc, 0x0000_0000);
}

#[test]
fn canonical_addi_x0_is_nop() {
    // ADDI x0, x0, 0 = 0x0000_0013 — the canonical NOP encoding
    let mut tc = ctx();
    decode_expect_nop(&mut tc, 0x0000_0013);
}

// ══════════════════════════════════════════════════════════
// 2. R-type integer ALU control signals
// ══════════════════════════════════════════════════════════

#[test]
fn add_sets_correct_signals() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().add(1, 2, 3).build();
    let id = decode_one(&mut tc, inst);

    assert_eq!(id.rd, 1);
    assert_eq!(id.rs1, 2);
    assert_eq!(id.rs2, 3);
    assert!(id.ctrl.reg_write, "ADD should write rd");
    assert!(!id.ctrl.mem_read, "ADD is not a load");
    assert!(!id.ctrl.mem_write, "ADD is not a store");
    assert!(!id.ctrl.branch, "ADD is not a branch");
    assert!(!id.ctrl.jump, "ADD is not a jump");
    assert!(matches!(id.ctrl.alu, AluOp::Add));
    assert!(matches!(id.ctrl.b_src, OpBSrc::Reg2), "R-type uses Reg2");
}

#[test]
fn sub_sets_correct_alu_op() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().sub(5, 6, 7).build();
    let id = decode_one(&mut tc, inst);

    assert!(matches!(id.ctrl.alu, AluOp::Sub));
    assert!(id.ctrl.reg_write);
    assert_eq!(id.rd, 5);
    assert_eq!(id.rs1, 6);
    assert_eq!(id.rs2, 7);
}

#[test]
fn and_or_xor_alu_ops() {
    let mut tc = ctx();

    let and_inst = InstructionBuilder::new().and(1, 2, 3).build();
    let id = decode_one(&mut tc, and_inst);
    assert!(matches!(id.ctrl.alu, AluOp::And));

    let or_inst = InstructionBuilder::new().or(1, 2, 3).build();
    let id = decode_one(&mut tc, or_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Or));

    let xor_inst = InstructionBuilder::new().xor(1, 2, 3).build();
    let id = decode_one(&mut tc, xor_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Xor));
}

#[test]
fn sll_srl_sra_alu_ops() {
    let mut tc = ctx();

    let sll_inst = InstructionBuilder::new().sll(1, 2, 3).build();
    let id = decode_one(&mut tc, sll_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Sll));

    let srl_inst = InstructionBuilder::new().srl(1, 2, 3).build();
    let id = decode_one(&mut tc, srl_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Srl));

    let sra_inst = InstructionBuilder::new().sra(1, 2, 3).build();
    let id = decode_one(&mut tc, sra_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Sra));
}

#[test]
fn slt_sltu_alu_ops() {
    let mut tc = ctx();

    let slt_inst = InstructionBuilder::new().slt(1, 2, 3).build();
    let id = decode_one(&mut tc, slt_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Slt));

    let sltu_inst = InstructionBuilder::new().sltu(1, 2, 3).build();
    let id = decode_one(&mut tc, sltu_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Sltu));
}

// ══════════════════════════════════════════════════════════
// 3. I-type immediate ALU control signals
// ══════════════════════════════════════════════════════════

#[test]
fn addi_sets_imm_source_and_add_op() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().addi(5, 10, 42).build();
    let id = decode_one(&mut tc, inst);

    assert_eq!(id.rd, 5);
    assert_eq!(id.rs1, 10);
    assert!(id.ctrl.reg_write);
    assert!(matches!(id.ctrl.alu, AluOp::Add));
    assert!(
        matches!(id.ctrl.b_src, OpBSrc::Imm),
        "I-type uses immediate"
    );
    assert_eq!(id.imm, 42, "Immediate should be 42");
}

#[test]
fn addi_negative_immediate() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().addi(1, 0, -1).build();
    let id = decode_one(&mut tc, inst);

    assert_eq!(id.imm, -1_i64, "Sign-extended negative immediate");
}

#[test]
fn andi_ori_xori_ops() {
    let mut tc = ctx();

    let andi_inst = InstructionBuilder::new().andi(1, 2, 0xFF).build();
    let id = decode_one(&mut tc, andi_inst);
    assert!(matches!(id.ctrl.alu, AluOp::And));
    assert!(matches!(id.ctrl.b_src, OpBSrc::Imm));

    let ori_inst = InstructionBuilder::new().ori(1, 2, 0xFF).build();
    let id = decode_one(&mut tc, ori_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Or));

    let xori_inst = InstructionBuilder::new().xori(1, 2, 0xFF).build();
    let id = decode_one(&mut tc, xori_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Xor));
}

#[test]
fn slti_sltiu_ops() {
    let mut tc = ctx();

    let slti_inst = InstructionBuilder::new().slti(1, 2, 10).build();
    let id = decode_one(&mut tc, slti_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Slt));
    assert!(matches!(id.ctrl.b_src, OpBSrc::Imm));

    let sltiu_inst = InstructionBuilder::new().sltiu(1, 2, 10).build();
    let id = decode_one(&mut tc, sltiu_inst);
    assert!(matches!(id.ctrl.alu, AluOp::Sltu));
}

// ══════════════════════════════════════════════════════════
// 4. RV64 32-bit variant control signals (OP-IMM-32, OP-REG-32)
// ══════════════════════════════════════════════════════════

#[test]
fn addiw_sets_rv32_flag() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().addiw(1, 2, 10).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.reg_write);
    assert!(id.ctrl.is_rv32, "ADDIW should set is_rv32");
    assert!(matches!(id.ctrl.alu, AluOp::Add));
}

#[test]
fn addw_subw_set_rv32_flag() {
    let mut tc = ctx();

    let addw_inst = InstructionBuilder::new().addw(1, 2, 3).build();
    let id = decode_one(&mut tc, addw_inst);
    assert!(id.ctrl.is_rv32, "ADDW should set is_rv32");
    assert!(matches!(id.ctrl.alu, AluOp::Add));

    let subw_inst = InstructionBuilder::new().subw(1, 2, 3).build();
    let id = decode_one(&mut tc, subw_inst);
    assert!(id.ctrl.is_rv32, "SUBW should set is_rv32");
    assert!(matches!(id.ctrl.alu, AluOp::Sub));
}

// ══════════════════════════════════════════════════════════
// 5. Load control signals and memory widths
// ══════════════════════════════════════════════════════════

#[test]
fn lw_sets_load_signals() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().lw(1, 2, 16).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.reg_write, "LW writes rd");
    assert!(id.ctrl.mem_read, "LW reads memory");
    assert!(!id.ctrl.mem_write, "LW does not write memory");
    assert!(matches!(id.ctrl.width, MemWidth::Word));
    assert!(id.ctrl.signed_load, "LW is sign-extending");
    assert!(matches!(id.ctrl.alu, AluOp::Add), "Address = rs1 + imm");
    assert_eq!(id.imm, 16);
}

#[test]
fn ld_sets_doubleword_width() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().ld(1, 2, 0).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.mem_read);
    assert!(matches!(id.ctrl.width, MemWidth::Double));
    assert!(id.ctrl.signed_load);
}

#[test]
fn load_immediate_is_address_offset() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().lw(1, 2, -4).build();
    let id = decode_one(&mut tc, inst);

    assert_eq!(id.imm, -4_i64, "Load offset should be sign-extended -4");
    assert!(
        matches!(id.ctrl.b_src, OpBSrc::Imm),
        "Load uses immediate for address calc"
    );
}

// ══════════════════════════════════════════════════════════
// 6. Store control signals and memory widths
// ══════════════════════════════════════════════════════════

#[test]
fn sw_sets_store_signals() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().sw(2, 3, 8).build();
    let id = decode_one(&mut tc, inst);

    assert!(!id.ctrl.reg_write, "SW does not write rd");
    assert!(!id.ctrl.mem_read, "SW does not read memory");
    assert!(id.ctrl.mem_write, "SW writes memory");
    assert!(matches!(id.ctrl.width, MemWidth::Word));
    assert!(matches!(id.ctrl.alu, AluOp::Add), "Address = rs1 + imm");
    assert!(
        matches!(id.ctrl.b_src, OpBSrc::Imm),
        "Store address uses immediate"
    );
}

#[test]
fn sd_sets_doubleword_width() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().sd(2, 3, 0).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.mem_write);
    assert!(matches!(id.ctrl.width, MemWidth::Double));
}

// ══════════════════════════════════════════════════════════
// 7. Branch control signals
// ══════════════════════════════════════════════════════════

#[test]
fn beq_sets_branch_flag() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.branch, "BEQ is a branch");
    assert!(!id.ctrl.jump, "BEQ is not a jump");
    assert!(!id.ctrl.reg_write, "BEQ does not write rd");
    assert!(
        matches!(id.ctrl.b_src, OpBSrc::Reg2),
        "Branch compares two registers"
    );
}

#[test]
fn bne_sets_branch_flag() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bne(3, 4, -4).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.branch);
    assert!(!id.ctrl.jump);
}

#[test]
fn all_branch_variants_set_branch_flag() {
    let mut tc = ctx();
    let variants = [
        InstructionBuilder::new().beq(1, 2, 8).build(),
        InstructionBuilder::new().bne(1, 2, 8).build(),
        InstructionBuilder::new().blt(1, 2, 8).build(),
        InstructionBuilder::new().bge(1, 2, 8).build(),
        InstructionBuilder::new().bltu(1, 2, 8).build(),
        InstructionBuilder::new().bgeu(1, 2, 8).build(),
    ];
    for inst in variants {
        let id = decode_one(&mut tc, inst);
        assert!(
            id.ctrl.branch,
            "Branch variant {:#010x} should set branch flag",
            inst
        );
        assert!(!id.ctrl.jump);
        assert!(!id.ctrl.reg_write);
    }
}

#[test]
fn branch_immediate_is_sign_extended() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bne(1, 2, -4).build();
    let id = decode_one(&mut tc, inst);

    // B-type immediate is a signed offset (always even)
    assert!(id.imm < 0, "Backward branch should have negative immediate");
}

// ══════════════════════════════════════════════════════════
// 8. Jump control signals (JAL, JALR)
// ══════════════════════════════════════════════════════════

#[test]
fn jal_sets_jump_and_reg_write() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().jal(1, 12).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.jump, "JAL is a jump");
    assert!(id.ctrl.reg_write, "JAL writes link register");
    assert!(!id.ctrl.branch, "JAL is not a branch");
    assert_eq!(id.rd, 1, "Link register is x1");
}

#[test]
fn jalr_sets_jump_and_reg_write() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().jalr(1, 5, 0).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.jump, "JALR is a jump");
    assert!(id.ctrl.reg_write, "JALR writes link register");
    assert!(!id.ctrl.branch);
    assert_eq!(id.rd, 1);
    assert_eq!(id.rs1, 5, "JALR base register");
}

// ══════════════════════════════════════════════════════════
// 9. Upper-immediate instructions (LUI, AUIPC)
// ══════════════════════════════════════════════════════════

#[test]
fn lui_sets_zero_a_source() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().lui(5, 0x12345).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.reg_write, "LUI writes rd");
    assert!(matches!(id.ctrl.a_src, OpASrc::Zero), "LUI uses Zero + imm");
    assert!(matches!(id.ctrl.alu, AluOp::Add));
    assert_eq!(id.rd, 5);
}

#[test]
fn auipc_sets_pc_a_source() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().auipc(5, 0x1).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.reg_write, "AUIPC writes rd");
    assert!(matches!(id.ctrl.a_src, OpASrc::Pc), "AUIPC uses PC + imm");
    assert!(matches!(id.ctrl.alu, AluOp::Add));
}

// ══════════════════════════════════════════════════════════
// 10. Register file reads
// ══════════════════════════════════════════════════════════

#[test]
fn decode_reads_rs1_from_register_file() {
    let mut tc = ctx();
    tc.set_reg(5, 0xDEAD_BEEF);
    let inst = InstructionBuilder::new().addi(1, 5, 0).build();
    let id = decode_one(&mut tc, inst);

    assert_eq!(id.rv1, 0xDEAD_BEEF, "rv1 should read x5 from register file");
}

#[test]
fn decode_reads_rs2_from_register_file() {
    let mut tc = ctx();
    tc.set_reg(7, 0xCAFE_BABE);
    let inst = InstructionBuilder::new().add(1, 0, 7).build();
    let id = decode_one(&mut tc, inst);

    assert_eq!(id.rv2, 0xCAFE_BABE, "rv2 should read x7 from register file");
}

#[test]
fn decode_reads_both_sources() {
    let mut tc = ctx();
    tc.set_reg(10, 100);
    tc.set_reg(11, 200);
    let inst = InstructionBuilder::new().add(1, 10, 11).build();
    let id = decode_one(&mut tc, inst);

    assert_eq!(id.rv1, 100, "rv1 = x10");
    assert_eq!(id.rv2, 200, "rv2 = x11");
}

#[test]
fn x0_always_reads_zero() {
    let mut tc = ctx();
    // x0 is hardwired to 0
    let inst = InstructionBuilder::new().add(1, 0, 0).build();
    let id = decode_one(&mut tc, inst);

    assert_eq!(id.rv1, 0, "x0 should always read as 0");
    assert_eq!(id.rv2, 0, "x0 should always read as 0");
}

// ══════════════════════════════════════════════════════════
// 11. Fetch-trap propagation
// ══════════════════════════════════════════════════════════

#[test]
fn fetch_trap_propagates_to_id_ex() {
    let mut tc = ctx();
    let trap = riscv_core::common::error::Trap::InstructionAccessFault(0xBAD);
    tc.cpu.if_id.entries = vec![IfIdEntry {
        pc: 0xBAD,
        inst: 0,
        inst_size: 4,
        pred_taken: false,
        pred_target: 0,
        trap: Some(trap.clone()),
    }];

    decode_stage(&mut tc.cpu);

    assert_eq!(tc.cpu.id_ex.entries.len(), 1, "Trap entry should propagate");
    let id = &tc.cpu.id_ex.entries[0];
    assert!(id.trap.is_some(), "Trap should be present in id_ex");
    assert_eq!(id.pc, 0xBAD, "PC should be preserved");
}

// ══════════════════════════════════════════════════════════
// 12. Illegal instruction trap generation
// ══════════════════════════════════════════════════════════

#[test]
fn illegal_opcode_generates_trap() {
    let mut tc = ctx();
    // Craft an instruction with an invalid opcode (0b1111111 = 0x7F is not a valid RV opcode)
    let bad_inst: u32 = 0x0000_007F;
    let id = decode_one(&mut tc, bad_inst);

    assert!(
        id.trap.is_some(),
        "Invalid opcode should generate IllegalInstruction trap"
    );
}

#[test]
fn illegal_load_funct3_generates_trap() {
    let mut tc = ctx();
    // OP_LOAD with funct3 = 0b111 is not a valid load variant
    let bad_inst = InstructionBuilder::new()
        .opcode(riscv_core::isa::rv64i::opcodes::OP_LOAD)
        .rd(1)
        .rs1(2)
        .funct3(0b111)
        .imm(0)
        .build();
    let id = decode_one(&mut tc, bad_inst);

    assert!(
        id.trap.is_some(),
        "Invalid load funct3 should generate IllegalInstruction trap"
    );
}

// ══════════════════════════════════════════════════════════
// 13. Prediction metadata forwarding
// ══════════════════════════════════════════════════════════

#[test]
fn branch_prediction_metadata_forwarded() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();

    tc.cpu.if_id.entries = vec![IfIdEntry {
        pc: 0x8000_0000,
        inst,
        inst_size: 4,
        pred_taken: true,
        pred_target: 0x8000_0008,
        trap: None,
    }];

    decode_stage(&mut tc.cpu);

    assert!(!tc.cpu.id_ex.entries.is_empty());
    let id = &tc.cpu.id_ex.entries[0];
    assert!(id.pred_taken, "pred_taken should be forwarded from IF/ID");
    assert_eq!(
        id.pred_target, 0x8000_0008,
        "pred_target should be forwarded from IF/ID"
    );
}

// ══════════════════════════════════════════════════════════
// 14. Multiple instructions in one cycle (superscalar)
// ══════════════════════════════════════════════════════════

#[test]
fn multiple_independent_instructions_decoded() {
    let mut tc = ctx();
    tc.set_reg(2, 100);
    tc.set_reg(4, 200);

    let inst0 = InstructionBuilder::new().addi(1, 2, 10).build(); // x1 = x2 + 10
    let inst1 = InstructionBuilder::new().addi(3, 4, 20).build(); // x3 = x4 + 20

    tc.cpu.if_id.entries = vec![
        IfIdEntry {
            pc: 0x8000_0000,
            inst: inst0,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
        IfIdEntry {
            pc: 0x8000_0004,
            inst: inst1,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
    ];

    decode_stage(&mut tc.cpu);

    assert_eq!(
        tc.cpu.id_ex.entries.len(),
        2,
        "Two independent instructions should both decode"
    );
    let id0 = &tc.cpu.id_ex.entries[0];
    let id1 = &tc.cpu.id_ex.entries[1];
    assert_eq!(id0.rd, 1);
    assert_eq!(id0.rv1, 100);
    assert_eq!(id1.rd, 3);
    assert_eq!(id1.rv1, 200);
}

#[test]
fn intra_bundle_raw_hazard_stalls_second() {
    let mut tc = ctx();
    // inst0: x1 = x0 + 10  (writes x1)
    // inst1: x2 = x1 + 20  (reads x1 — RAW hazard within same bundle)
    let inst0 = InstructionBuilder::new().addi(1, 0, 10).build();
    let inst1 = InstructionBuilder::new().addi(2, 1, 20).build();

    tc.cpu.if_id.entries = vec![
        IfIdEntry {
            pc: 0x8000_0000,
            inst: inst0,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
        IfIdEntry {
            pc: 0x8000_0004,
            inst: inst1,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
    ];

    decode_stage(&mut tc.cpu);

    // Only the first instruction should decode; the second should remain
    // in if_id because of the intra-bundle hazard.
    assert_eq!(
        tc.cpu.id_ex.entries.len(),
        1,
        "Hazardous second instruction should NOT decode in this cycle"
    );
    assert_eq!(tc.cpu.id_ex.entries[0].rd, 1, "First instruction decoded");

    // The second instruction should remain in if_id for the next cycle.
    assert!(
        !tc.cpu.if_id.entries.is_empty(),
        "Stalled instruction should remain in if_id"
    );
}

#[test]
fn intra_bundle_no_hazard_on_different_registers() {
    let mut tc = ctx();
    // inst0: x1 = x0 + 10  (writes x1)
    // inst1: x3 = x2 + 20  (reads x2 — no dependency on x1)
    let inst0 = InstructionBuilder::new().addi(1, 0, 10).build();
    let inst1 = InstructionBuilder::new().addi(3, 2, 20).build();

    tc.cpu.if_id.entries = vec![
        IfIdEntry {
            pc: 0x8000_0000,
            inst: inst0,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
        IfIdEntry {
            pc: 0x8000_0004,
            inst: inst1,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
    ];

    decode_stage(&mut tc.cpu);

    assert_eq!(
        tc.cpu.id_ex.entries.len(),
        2,
        "No hazard — both should decode"
    );
}

#[test]
fn intra_bundle_hazard_on_rs2() {
    let mut tc = ctx();
    // inst0: x5 = x0 + 10  (writes x5)
    // inst1: ADD x6, x0, x5 (reads x5 as rs2 — RAW hazard)
    let inst0 = InstructionBuilder::new().addi(5, 0, 10).build();
    let inst1 = InstructionBuilder::new().add(6, 0, 5).build();

    tc.cpu.if_id.entries = vec![
        IfIdEntry {
            pc: 0x8000_0000,
            inst: inst0,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
        IfIdEntry {
            pc: 0x8000_0004,
            inst: inst1,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
    ];

    decode_stage(&mut tc.cpu);

    assert_eq!(
        tc.cpu.id_ex.entries.len(),
        1,
        "rs2 hazard should stall second instruction"
    );
}

#[test]
fn writes_to_x0_do_not_cause_hazard() {
    let mut tc = ctx();
    // inst0: ADDI x0, x0, 10  → writes x0 (which is always 0, no effect)
    // inst1: ADDI x1, x0, 20  → reads x0 (should NOT be treated as hazard)
    // Note: ADDI x0, ... is NOP and gets skipped, so use a different approach.
    // Use ADD x0, x2, x3 — R-type that writes to x0.
    let inst0 = InstructionBuilder::new().add(0, 2, 3).build();
    let inst1 = InstructionBuilder::new().addi(1, 0, 20).build();

    tc.cpu.if_id.entries = vec![
        IfIdEntry {
            pc: 0x8000_0000,
            inst: inst0,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
        IfIdEntry {
            pc: 0x8000_0004,
            inst: inst1,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
    ];

    decode_stage(&mut tc.cpu);

    // ADD x0 writes to x0 (no effect), and ADDI reading x0 should NOT hazard
    // because x0 writes are ignored for hazard purposes.
    // The first inst (ADD x0,...) has ctrl.reg_write=true and rd=0.
    // The decode's hazard check looks at bundle_writes: rd=0 IS pushed to
    // bundle_writes since ctrl.reg_write && d.rd != 0 is false (rd==0).
    // Actually, let me check: bundle_writes.push((d.rd, false)) only when
    // ctrl.reg_write && d.rd != 0. So x0 is NOT pushed → no hazard. Good.
    assert_eq!(
        tc.cpu.id_ex.entries.len(),
        2,
        "Write to x0 should not cause an intra-bundle hazard"
    );
}

// ══════════════════════════════════════════════════════════
// 15. PC is correctly forwarded to ID/EX
// ══════════════════════════════════════════════════════════

#[test]
fn pc_and_inst_size_forwarded() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().addi(1, 0, 5).build();

    tc.cpu.if_id.entries = vec![IfIdEntry {
        pc: 0x8000_1234,
        inst,
        inst_size: 4,
        pred_taken: false,
        pred_target: 0,
        trap: None,
    }];

    decode_stage(&mut tc.cpu);

    let id = &tc.cpu.id_ex.entries[0];
    assert_eq!(id.pc, 0x8000_1234, "PC should be forwarded");
    assert_eq!(id.inst_size, 4, "inst_size should be forwarded");
    assert_eq!(id.inst, inst, "Raw instruction should be forwarded");
}

// ══════════════════════════════════════════════════════════
// 16. Mixed NOP and real instructions
// ══════════════════════════════════════════════════════════

#[test]
fn nop_between_real_instructions_is_skipped() {
    let mut tc = ctx();
    let nop = InstructionBuilder::new().nop().build();
    let real = InstructionBuilder::new().addi(1, 0, 42).build();

    tc.cpu.if_id.entries = vec![
        IfIdEntry {
            pc: 0x8000_0000,
            inst: nop,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
        IfIdEntry {
            pc: 0x8000_0004,
            inst: real,
            inst_size: 4,
            pred_taken: false,
            pred_target: 0,
            trap: None,
        },
    ];

    decode_stage(&mut tc.cpu);

    // NOP is skipped, real instruction should decode
    assert_eq!(
        tc.cpu.id_ex.entries.len(),
        1,
        "Only the real instruction should appear in id_ex"
    );
    assert_eq!(tc.cpu.id_ex.entries[0].rd, 1);
    assert_eq!(tc.cpu.id_ex.entries[0].imm, 42);
}

// ══════════════════════════════════════════════════════════
// 17. Store reads rs2 for store data
// ══════════════════════════════════════════════════════════

#[test]
fn store_reads_rs2_for_data() {
    let mut tc = ctx();
    tc.set_reg(3, 0xAABBCCDD);
    let inst = InstructionBuilder::new().sw(2, 3, 0).build(); // SW rs1=x2, rs2=x3

    let id = decode_one(&mut tc, inst);

    assert_eq!(id.rs2, 3, "Store data register is rs2");
    assert_eq!(id.rv2, 0xAABBCCDD, "rv2 should hold the store data");
}

// ══════════════════════════════════════════════════════════
// 18. Branch/Jump do not set reg_write (branches) or do (jumps)
// ══════════════════════════════════════════════════════════

#[test]
fn branch_does_not_write_register() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();
    let id = decode_one(&mut tc, inst);

    assert!(
        !id.ctrl.reg_write,
        "Branches should not write to a register"
    );
    assert!(!id.ctrl.fp_reg_write);
}

#[test]
fn jal_writes_link_register() {
    let mut tc = ctx();
    // JAL x1, +12 → x1 = PC + 4 (set during writeback, but decode sets reg_write)
    let inst = InstructionBuilder::new().jal(1, 12).build();
    let id = decode_one(&mut tc, inst);

    assert!(id.ctrl.reg_write, "JAL writes the link register");
    assert!(id.ctrl.jump);
    assert_eq!(id.rd, 1);
}

// ══════════════════════════════════════════════════════════
// 19. Empty IF/ID produces empty ID/EX
// ══════════════════════════════════════════════════════════

#[test]
fn empty_if_id_produces_empty_id_ex() {
    let mut tc = ctx();
    tc.cpu.if_id.entries.clear();

    decode_stage(&mut tc.cpu);

    assert!(
        tc.cpu.id_ex.entries.is_empty(),
        "Empty IF/ID should produce empty ID/EX"
    );
}
