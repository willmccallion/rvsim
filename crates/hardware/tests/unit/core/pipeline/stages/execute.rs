//! Execute Stage Unit Tests.
//!
//! Verifies that `execute_stage` correctly performs:
//!   1. ALU dispatch — correct operation with proper operand muxing
//!   2. Operand source selection — Reg1/Pc/Zero for A, Reg2/Imm/Zero for B
//!   3. Branch resolution — all 6 funct3 variants, taken / not-taken
//!   4. Branch mispredict detection — flush IF/ID, redirect PC, update stats
//!   5. Branch correct-prediction — update prediction stats
//!   6. JAL target computation — PC + imm, link register written
//!   7. JALR target computation — (rs1 + imm) & ~1, link register
//!   8. Jump mispredict detection and pipeline flush
//!   9. Trap propagation without ALU execution
//!  10. Store data routing (store_data = forwarded rs2)
//!  11. Multiple entries and flush-remaining semantics

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use riscv_core::core::pipeline::latches::{IdExEntry, IfIdEntry};
use riscv_core::core::pipeline::signals::{AluOp, ControlSignals, MemWidth, OpASrc, OpBSrc};
use riscv_core::core::pipeline::stages::execute_stage;

// ══════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════

const PC: u64 = 0x8000_0000;
const INST_SIZE: u64 = 4;

fn ctx() -> TestContext {
    TestContext::new()
}

/// Build an IdExEntry for an integer ALU operation (R-type style).
fn alu_entry(alu: AluOp, rv1: u64, rv2: u64, rd: usize) -> IdExEntry {
    IdExEntry {
        pc: PC,
        inst: InstructionBuilder::new().add(rd as u32, 0, 0).build(), // placeholder encoding
        inst_size: INST_SIZE,
        rs1: 1,
        rs2: 2,
        rd,
        rv1,
        rv2,
        ctrl: ControlSignals {
            reg_write: true,
            alu,
            a_src: OpASrc::Reg1,
            b_src: OpBSrc::Reg2,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Build an IdExEntry for a branch instruction.
/// Uses real instruction encoding so funct3 is correct for execute_stage.
fn branch_entry(
    inst: u32,
    rs1_val: u64,
    rs2_val: u64,
    imm: i64,
    pred_taken: bool,
    pred_target: u64,
) -> IdExEntry {
    IdExEntry {
        pc: PC,
        inst,
        inst_size: INST_SIZE,
        rs1: 1,
        rs2: 2,
        rd: 0,
        imm,
        rv1: rs1_val,
        rv2: rs2_val,
        ctrl: ControlSignals {
            branch: true,
            a_src: OpASrc::Reg1,
            b_src: OpBSrc::Reg2,
            ..Default::default()
        },
        pred_taken,
        pred_target,
        ..Default::default()
    }
}

/// Build an IdExEntry for a JAL instruction.
fn jal_entry(rd: usize, imm: i64, pred_taken: bool, pred_target: u64) -> IdExEntry {
    IdExEntry {
        pc: PC,
        inst: InstructionBuilder::new().jal(rd as u32, imm as i32).build(),
        inst_size: INST_SIZE,
        rd,
        imm,
        ctrl: ControlSignals {
            jump: true,
            reg_write: true,
            ..Default::default()
        },
        pred_taken,
        pred_target,
        ..Default::default()
    }
}

/// Build an IdExEntry for a JALR instruction.
fn jalr_entry(
    rd: usize,
    rs1: usize,
    rs1_val: u64,
    imm: i64,
    pred_taken: bool,
    pred_target: u64,
) -> IdExEntry {
    IdExEntry {
        pc: PC,
        inst: InstructionBuilder::new()
            .jalr(rd as u32, rs1 as u32, imm as i32)
            .build(),
        inst_size: INST_SIZE,
        rs1,
        rd,
        imm,
        rv1: rs1_val,
        ctrl: ControlSignals {
            jump: true,
            reg_write: true,
            ..Default::default()
        },
        pred_taken,
        pred_target,
        ..Default::default()
    }
}

/// Run execute_stage with a single IdExEntry and return the first ExMemEntry.
fn exec_one(
    tc: &mut TestContext,
    entry: IdExEntry,
) -> riscv_core::core::pipeline::latches::ExMemEntry {
    tc.cpu.id_ex.entries = vec![entry];
    execute_stage(&mut tc.cpu);
    assert!(
        !tc.cpu.ex_mem.entries.is_empty(),
        "execute_stage should produce at least one EX/MEM entry"
    );
    tc.cpu.ex_mem.entries.remove(0)
}

// ══════════════════════════════════════════════════════════
// 1. ALU dispatch — correct operations
// ══════════════════════════════════════════════════════════

#[test]
fn alu_add_computes_sum() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Add, 100, 42, 5));
    assert_eq!(ex.alu, 142, "ADD: 100 + 42 = 142");
    assert_eq!(ex.rd, 5);
}

#[test]
fn alu_sub_computes_difference() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Sub, 100, 42, 5));
    assert_eq!(ex.alu, 58, "SUB: 100 - 42 = 58");
}

#[test]
fn alu_and_computes_bitwise_and() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::And, 0xFF00, 0x0FF0, 1));
    assert_eq!(ex.alu, 0x0F00);
}

#[test]
fn alu_or_computes_bitwise_or() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Or, 0xFF00, 0x0FF0, 1));
    assert_eq!(ex.alu, 0xFFF0);
}

#[test]
fn alu_xor_computes_bitwise_xor() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Xor, 0xFF00, 0x0FF0, 1));
    assert_eq!(ex.alu, 0xF0F0);
}

#[test]
fn alu_sll_shifts_left() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Sll, 1, 4, 1));
    assert_eq!(ex.alu, 16, "SLL: 1 << 4 = 16");
}

#[test]
fn alu_srl_shifts_right_logical() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Srl, 0x80, 3, 1));
    assert_eq!(ex.alu, 0x10, "SRL: 0x80 >> 3 = 0x10");
}

#[test]
fn alu_sra_shifts_right_arithmetic() {
    let mut tc = ctx();
    let neg = (-16_i64) as u64;
    let ex = exec_one(&mut tc, alu_entry(AluOp::Sra, neg, 2, 1));
    assert_eq!(ex.alu as i64, -4, "SRA: -16 >> 2 = -4");
}

#[test]
fn alu_slt_signed_comparison() {
    let mut tc = ctx();
    let neg1 = (-1_i64) as u64;
    let ex = exec_one(&mut tc, alu_entry(AluOp::Slt, neg1, 0, 1));
    assert_eq!(ex.alu, 1, "SLT: -1 < 0 → 1");

    let ex2 = exec_one(&mut tc, alu_entry(AluOp::Slt, 0, neg1, 1));
    assert_eq!(ex2.alu, 0, "SLT: 0 < -1 → 0");
}

#[test]
fn alu_sltu_unsigned_comparison() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Sltu, 5, 10, 1));
    assert_eq!(ex.alu, 1, "SLTU: 5 <u 10 → 1");

    let big = u64::MAX;
    let ex2 = exec_one(&mut tc, alu_entry(AluOp::Sltu, big, 10, 1));
    assert_eq!(ex2.alu, 0, "SLTU: MAX <u 10 → 0");
}

#[test]
fn alu_mul_low_bits() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Mul, 7, 6, 1));
    assert_eq!(ex.alu, 42, "MUL: 7 × 6 = 42");
}

#[test]
fn alu_div_signed() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Div, 42, 6, 1));
    assert_eq!(ex.alu, 7, "DIV: 42 / 6 = 7");
}

#[test]
fn alu_divu_unsigned() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Divu, 42, 6, 1));
    assert_eq!(ex.alu, 7, "DIVU: 42 / 6 = 7");
}

#[test]
fn alu_rem_signed() {
    let mut tc = ctx();
    let ex = exec_one(&mut tc, alu_entry(AluOp::Rem, 17, 5, 1));
    assert_eq!(ex.alu, 2, "REM: 17 % 5 = 2");
}

// ══════════════════════════════════════════════════════════
// 2. Operand source selection (OpASrc / OpBSrc)
// ══════════════════════════════════════════════════════════

#[test]
fn op_a_src_pc() {
    let mut tc = ctx();
    let entry = IdExEntry {
        pc: 0x8000_0100,
        inst: InstructionBuilder::new().auipc(1, 0x1).build(),
        inst_size: INST_SIZE,
        rd: 1,
        imm: 0x1000,
        ctrl: ControlSignals {
            reg_write: true,
            alu: AluOp::Add,
            a_src: OpASrc::Pc,
            b_src: OpBSrc::Imm,
            ..Default::default()
        },
        ..Default::default()
    };
    let ex = exec_one(&mut tc, entry);
    assert_eq!(ex.alu, 0x8000_0100 + 0x1000, "AUIPC: PC + imm");
}

#[test]
fn op_a_src_zero() {
    let mut tc = ctx();
    let entry = IdExEntry {
        pc: PC,
        inst: InstructionBuilder::new().lui(5, 0x12345).build(),
        inst_size: INST_SIZE,
        rd: 5,
        rv1: 0xDEAD, // should be ignored due to Zero source
        imm: 0x12345000,
        ctrl: ControlSignals {
            reg_write: true,
            alu: AluOp::Add,
            a_src: OpASrc::Zero,
            b_src: OpBSrc::Imm,
            ..Default::default()
        },
        ..Default::default()
    };
    let ex = exec_one(&mut tc, entry);
    assert_eq!(ex.alu, 0x12345000, "LUI: 0 + imm");
}

#[test]
fn op_b_src_imm() {
    let mut tc = ctx();
    let entry = IdExEntry {
        pc: PC,
        inst: InstructionBuilder::new().addi(1, 2, 42).build(),
        inst_size: INST_SIZE,
        rs1: 2,
        rd: 1,
        rv1: 100,
        rv2: 0xDEAD, // should be ignored
        imm: 42,
        ctrl: ControlSignals {
            reg_write: true,
            alu: AluOp::Add,
            a_src: OpASrc::Reg1,
            b_src: OpBSrc::Imm,
            ..Default::default()
        },
        ..Default::default()
    };
    let ex = exec_one(&mut tc, entry);
    assert_eq!(ex.alu, 142, "ADDI: rs1 + imm = 100 + 42");
}

#[test]
fn op_b_src_zero() {
    let mut tc = ctx();
    let entry = IdExEntry {
        pc: PC,
        inst: InstructionBuilder::new().add(1, 2, 0).build(),
        inst_size: INST_SIZE,
        rs1: 2,
        rd: 1,
        rv1: 77,
        rv2: 0xDEAD, // should be ignored
        ctrl: ControlSignals {
            reg_write: true,
            alu: AluOp::Add,
            a_src: OpASrc::Reg1,
            b_src: OpBSrc::Zero,
            ..Default::default()
        },
        ..Default::default()
    };
    let ex = exec_one(&mut tc, entry);
    assert_eq!(ex.alu, 77, "Add with Zero B: rs1 + 0 = 77");
}

// ══════════════════════════════════════════════════════════
// 3. Branch resolution — all 6 variants (taken / not-taken)
// ══════════════════════════════════════════════════════════

#[test]
fn beq_taken_when_equal() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();
    // Predict not-taken (default), branch is actually taken → mispredict
    let entry = branch_entry(inst, 42, 42, 8, false, 0);
    exec_one(&mut tc, entry);

    let target = PC.wrapping_add(8);
    assert_eq!(tc.cpu.pc, target, "BEQ taken → PC = PC + 8");
    assert!(
        tc.cpu.if_id.entries.is_empty(),
        "IF/ID flushed on mispredict"
    );
    assert_eq!(tc.cpu.stats.branch_mispredictions, 1);
}

#[test]
fn beq_not_taken_when_unequal() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();
    // Predict not-taken, branch actually not taken → correct
    let entry = branch_entry(inst, 10, 20, 8, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(
        tc.cpu.stats.branch_predictions, 1,
        "Correct prediction counted"
    );
    assert_eq!(tc.cpu.stats.branch_mispredictions, 0);
}

#[test]
fn bne_taken_when_unequal() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bne(1, 2, 16).build();
    let entry = branch_entry(inst, 10, 20, 16, false, 0);
    exec_one(&mut tc, entry);

    let target = PC.wrapping_add(16);
    assert_eq!(tc.cpu.pc, target, "BNE taken → PC = PC + 16");
    assert_eq!(tc.cpu.stats.branch_mispredictions, 1);
}

#[test]
fn bne_not_taken_when_equal() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bne(1, 2, 16).build();
    let entry = branch_entry(inst, 42, 42, 16, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.stats.branch_predictions, 1);
    assert_eq!(tc.cpu.stats.branch_mispredictions, 0);
}

#[test]
fn blt_taken_when_less_signed() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().blt(1, 2, 12).build();
    let neg = (-5_i64) as u64;
    let entry = branch_entry(inst, neg, 10, 12, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.pc, PC.wrapping_add(12), "BLT: -5 < 10 → taken");
    assert_eq!(tc.cpu.stats.branch_mispredictions, 1);
}

#[test]
fn blt_not_taken_when_ge() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().blt(1, 2, 12).build();
    let entry = branch_entry(inst, 10, 5, 12, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.stats.branch_predictions, 1);
}

#[test]
fn bge_taken_when_ge() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bge(1, 2, 8).build();
    let entry = branch_entry(inst, 10, 10, 8, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.pc, PC.wrapping_add(8), "BGE: 10 >= 10 → taken");
    assert_eq!(tc.cpu.stats.branch_mispredictions, 1);
}

#[test]
fn bge_not_taken_when_lt() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bge(1, 2, 8).build();
    let entry = branch_entry(inst, 5, 10, 8, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.stats.branch_predictions, 1);
}

#[test]
fn bltu_taken_when_less_unsigned() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bltu(1, 2, 8).build();
    let entry = branch_entry(inst, 5, u64::MAX, 8, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.pc, PC.wrapping_add(8), "BLTU: 5 <u MAX → taken");
    assert_eq!(tc.cpu.stats.branch_mispredictions, 1);
}

#[test]
fn bltu_not_taken_when_geu() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bltu(1, 2, 8).build();
    let entry = branch_entry(inst, u64::MAX, 5, 8, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.stats.branch_predictions, 1);
}

#[test]
fn bgeu_taken_when_geu() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bgeu(1, 2, 8).build();
    let entry = branch_entry(inst, u64::MAX, 5, 8, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.pc, PC.wrapping_add(8), "BGEU: MAX >=u 5 → taken");
    assert_eq!(tc.cpu.stats.branch_mispredictions, 1);
}

#[test]
fn bgeu_not_taken_when_ltu() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bgeu(1, 2, 8).build();
    let entry = branch_entry(inst, 5, 10, 8, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.stats.branch_predictions, 1);
}

// ══════════════════════════════════════════════════════════
// 4. Branch mispredict vs correct prediction mechanics
// ══════════════════════════════════════════════════════════

#[test]
fn branch_taken_predicted_taken_correct_target_is_hit() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();
    let target = PC.wrapping_add(8);
    // Predict taken with correct target → no mispredict
    let entry = branch_entry(inst, 42, 42, 8, true, target);
    exec_one(&mut tc, entry);

    assert_eq!(
        tc.cpu.stats.branch_predictions, 1,
        "Correct taken prediction"
    );
    assert_eq!(tc.cpu.stats.branch_mispredictions, 0);
}

#[test]
fn branch_taken_predicted_taken_wrong_target_is_mispredict() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();
    // Predict taken but to the wrong target
    let entry = branch_entry(inst, 42, 42, 8, true, 0xBAAD);
    exec_one(&mut tc, entry);

    assert_eq!(
        tc.cpu.stats.branch_mispredictions, 1,
        "Wrong target → mispredict"
    );
    assert_eq!(
        tc.cpu.pc,
        PC.wrapping_add(8),
        "PC corrected to actual target"
    );
}

#[test]
fn branch_not_taken_predicted_taken_is_mispredict() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();
    let target = PC.wrapping_add(8);
    // Predict taken, but branch is not taken (10 != 20)
    let entry = branch_entry(inst, 10, 20, 8, true, target);
    exec_one(&mut tc, entry);

    let fallthrough = PC.wrapping_add(INST_SIZE);
    assert_eq!(tc.cpu.pc, fallthrough, "Mispredict → PC = fallthrough");
    assert_eq!(tc.cpu.stats.branch_mispredictions, 1);
}

#[test]
fn branch_mispredict_flushes_if_id() {
    let mut tc = ctx();
    // Pre-populate IF/ID with some dummy entries
    tc.cpu.if_id.entries = vec![IfIdEntry {
        pc: 0x1000,
        inst: 0x13,
        inst_size: 4,
        ..Default::default()
    }];
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();
    let entry = branch_entry(inst, 42, 42, 8, false, 0); // taken, predicted not-taken
    exec_one(&mut tc, entry);

    assert!(
        tc.cpu.if_id.entries.is_empty(),
        "IF/ID must be flushed on branch mispredict"
    );
}

#[test]
fn branch_mispredict_increments_stalls_control() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().beq(1, 2, 8).build();
    let entry = branch_entry(inst, 42, 42, 8, false, 0);
    exec_one(&mut tc, entry);

    assert_eq!(
        tc.cpu.stats.stalls_control, 2,
        "Mispredict adds 2 control stalls"
    );
}

#[test]
fn backward_branch_target_computed_correctly() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().bne(1, 2, -4).build();
    let entry = branch_entry(inst, 10, 20, -4, false, 0); // taken (10 != 20)
    exec_one(&mut tc, entry);

    let target = PC.wrapping_add((-4_i64) as u64);
    assert_eq!(tc.cpu.pc, target, "Backward branch: PC + (-4)");
}

// ══════════════════════════════════════════════════════════
// 5. JAL target and link
// ══════════════════════════════════════════════════════════

#[test]
fn jal_computes_pc_plus_imm() {
    let mut tc = ctx();
    // JAL x1, +20. No prediction → predicted fallthrough = PC+4.
    let entry = jal_entry(1, 20, false, 0);
    let ex = exec_one(&mut tc, entry);

    let target = PC.wrapping_add(20);
    assert_eq!(tc.cpu.pc, target, "JAL target = PC + imm");
    assert_eq!(
        tc.cpu.stats.branch_mispredictions, 1,
        "Unpredicted jump → mispredict"
    );
    assert!(ex.ctrl.jump, "ExMem should carry jump flag");
    assert!(ex.ctrl.reg_write, "JAL writes link register");
    assert_eq!(ex.rd, 1);
}

#[test]
fn jal_correctly_predicted() {
    let mut tc = ctx();
    let target = PC.wrapping_add(20);
    let entry = jal_entry(1, 20, true, target);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.stats.branch_predictions, 1);
    assert_eq!(tc.cpu.stats.branch_mispredictions, 0);
}

#[test]
fn jal_backward_jump() {
    let mut tc = ctx();
    let entry = jal_entry(1, -8, false, 0);
    exec_one(&mut tc, entry);

    let target = PC.wrapping_add((-8_i64) as u64);
    assert_eq!(tc.cpu.pc, target, "JAL backward: PC + (-8)");
}

// ══════════════════════════════════════════════════════════
// 6. JALR target and link
// ══════════════════════════════════════════════════════════

#[test]
fn jalr_computes_rs1_plus_imm_aligned() {
    let mut tc = ctx();
    // JALR x1, x5, 4. rs1_val = 0x8000_0010.
    let entry = jalr_entry(1, 5, 0x8000_0010, 4, false, 0);
    exec_one(&mut tc, entry);

    let target = (0x8000_0010_u64.wrapping_add(4)) & !1;
    assert_eq!(tc.cpu.pc, target, "JALR: (rs1 + imm) & ~1");
}

#[test]
fn jalr_clears_lowest_bit() {
    let mut tc = ctx();
    // Target before alignment would be 0x8000_0011 (odd)
    let entry = jalr_entry(1, 5, 0x8000_0010, 1, false, 0);
    exec_one(&mut tc, entry);

    let target = (0x8000_0010_u64.wrapping_add(1)) & !1; // 0x8000_0010
    assert_eq!(tc.cpu.pc, target, "JALR clears bit 0 for alignment");
}

#[test]
fn jalr_correctly_predicted() {
    let mut tc = ctx();
    let target = (0x8000_0010_u64.wrapping_add(0)) & !1;
    let entry = jalr_entry(1, 5, 0x8000_0010, 0, true, target);
    exec_one(&mut tc, entry);

    assert_eq!(tc.cpu.stats.branch_predictions, 1);
    assert_eq!(tc.cpu.stats.branch_mispredictions, 0);
}

#[test]
fn jalr_mispredict_flushes_pipeline() {
    let mut tc = ctx();
    tc.cpu.if_id.entries = vec![IfIdEntry::default()];

    let entry = jalr_entry(1, 5, 0x8000_0010, 0, false, 0);
    exec_one(&mut tc, entry);

    assert!(
        tc.cpu.if_id.entries.is_empty(),
        "IF/ID flushed on JALR mispredict"
    );
    assert_eq!(tc.cpu.stats.branch_mispredictions, 1);
}

// ══════════════════════════════════════════════════════════
// 7. Trap propagation
// ══════════════════════════════════════════════════════════

#[test]
fn trap_passes_through_without_alu() {
    let mut tc = ctx();
    let trap = riscv_core::common::error::Trap::IllegalInstruction(0xDEAD);
    let entry = IdExEntry {
        pc: PC,
        inst: 0xDEAD,
        inst_size: INST_SIZE,
        trap: Some(trap),
        ..Default::default()
    };
    let ex = exec_one(&mut tc, entry);

    assert!(ex.trap.is_some(), "Trap should propagate through EX stage");
    assert_eq!(ex.alu, 0, "ALU should not compute when trap is present");
    assert_eq!(ex.pc, PC, "PC preserved");
}

#[test]
fn trap_does_not_modify_cpu_pc() {
    let mut tc = ctx();
    let original_pc = tc.cpu.pc;
    let trap = riscv_core::common::error::Trap::IllegalInstruction(0xDEAD);
    let entry = IdExEntry {
        pc: PC,
        inst: 0xDEAD,
        inst_size: INST_SIZE,
        trap: Some(trap),
        ..Default::default()
    };
    exec_one(&mut tc, entry);

    assert_eq!(
        tc.cpu.pc, original_pc,
        "CPU PC should not change for trap propagation"
    );
}

// ══════════════════════════════════════════════════════════
// 8. Store data routing
// ══════════════════════════════════════════════════════════

#[test]
fn store_data_comes_from_rs2_value() {
    let mut tc = ctx();
    let entry = IdExEntry {
        pc: PC,
        inst: InstructionBuilder::new().sw(2, 3, 0).build(),
        inst_size: INST_SIZE,
        rs1: 2,
        rs2: 3,
        rv1: 0x8000_0000, // base address
        rv2: 0xDEAD_BEEF, // store data
        imm: 0,
        ctrl: ControlSignals {
            mem_write: true,
            alu: AluOp::Add,
            a_src: OpASrc::Reg1,
            b_src: OpBSrc::Imm,
            width: MemWidth::Word,
            ..Default::default()
        },
        ..Default::default()
    };
    let ex = exec_one(&mut tc, entry);

    assert_eq!(ex.store_data, 0xDEAD_BEEF, "store_data = forwarded rs2");
    assert_eq!(ex.alu, 0x8000_0000, "alu = base + offset = address");
}

// ══════════════════════════════════════════════════════════
// 9. Load address computation
// ══════════════════════════════════════════════════════════

#[test]
fn load_address_is_rs1_plus_imm() {
    let mut tc = ctx();
    let entry = IdExEntry {
        pc: PC,
        inst: InstructionBuilder::new().lw(1, 2, 16).build(),
        inst_size: INST_SIZE,
        rs1: 2,
        rd: 1,
        rv1: 0x8000_0000,
        imm: 16,
        ctrl: ControlSignals {
            reg_write: true,
            mem_read: true,
            alu: AluOp::Add,
            a_src: OpASrc::Reg1,
            b_src: OpBSrc::Imm,
            width: MemWidth::Word,
            signed_load: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let ex = exec_one(&mut tc, entry);

    assert_eq!(ex.alu, 0x8000_0010, "Load address = rs1 + imm");
}

// ══════════════════════════════════════════════════════════
// 10. Multiple entries and flush-remaining semantics
// ══════════════════════════════════════════════════════════

#[test]
fn entries_after_branch_mispredict_are_flushed() {
    let mut tc = ctx();

    let branch = branch_entry(
        InstructionBuilder::new().beq(1, 2, 8).build(),
        42,
        42, // equal → taken
        8,
        false,
        0, // predicted not-taken → mispredict
    );
    let should_flush = alu_entry(AluOp::Add, 100, 200, 5);

    tc.cpu.id_ex.entries = vec![branch, should_flush];
    execute_stage(&mut tc.cpu);

    // Only the branch should produce an EX/MEM entry; the second is flushed.
    assert_eq!(
        tc.cpu.ex_mem.entries.len(),
        1,
        "Second entry should be flushed after branch mispredict"
    );
}

#[test]
fn entries_after_jump_mispredict_are_flushed() {
    let mut tc = ctx();

    let jal = jal_entry(1, 20, false, 0); // unpredicted → mispredict
    let should_flush = alu_entry(AluOp::Add, 100, 200, 5);

    tc.cpu.id_ex.entries = vec![jal, should_flush];
    execute_stage(&mut tc.cpu);

    assert_eq!(
        tc.cpu.ex_mem.entries.len(),
        1,
        "Second entry should be flushed after jump mispredict"
    );
}

#[test]
fn multiple_non_branch_entries_all_execute() {
    let mut tc = ctx();

    // Use non-overlapping rd / rs registers to avoid intra-bundle forwarding.
    let e1 = alu_entry(AluOp::Add, 10, 20, 5); // rd=5, rs1=1, rs2=2
    let mut e2 = alu_entry(AluOp::Sub, 100, 30, 6); // rd=6
    e2.rs1 = 3; // different from e1.rd
    e2.rs2 = 4;

    tc.cpu.id_ex.entries = vec![e1, e2];
    execute_stage(&mut tc.cpu);

    assert_eq!(
        tc.cpu.ex_mem.entries.len(),
        2,
        "Both non-branch entries execute"
    );
    assert_eq!(tc.cpu.ex_mem.entries[0].alu, 30);
    assert_eq!(tc.cpu.ex_mem.entries[1].alu, 70);
}

// ══════════════════════════════════════════════════════════
// 11. Empty ID/EX produces empty EX/MEM
// ══════════════════════════════════════════════════════════

#[test]
fn empty_id_ex_produces_empty_ex_mem() {
    let mut tc = ctx();
    tc.cpu.id_ex.entries.clear();
    execute_stage(&mut tc.cpu);

    assert!(
        tc.cpu.ex_mem.entries.is_empty(),
        "Empty ID/EX → empty EX/MEM"
    );
}

// ══════════════════════════════════════════════════════════
// 12. PC and metadata forwarded to EX/MEM
// ══════════════════════════════════════════════════════════

#[test]
fn ex_mem_preserves_pc_and_inst() {
    let mut tc = ctx();
    let inst = InstructionBuilder::new().add(1, 2, 3).build();
    let entry = IdExEntry {
        pc: 0x8000_ABCD,
        inst,
        inst_size: 4,
        rs1: 2,
        rs2: 3,
        rd: 1,
        rv1: 10,
        rv2: 20,
        ctrl: ControlSignals {
            reg_write: true,
            alu: AluOp::Add,
            a_src: OpASrc::Reg1,
            b_src: OpBSrc::Reg2,
            ..Default::default()
        },
        ..Default::default()
    };
    let ex = exec_one(&mut tc, entry);

    assert_eq!(ex.pc, 0x8000_ABCD, "PC forwarded");
    assert_eq!(ex.inst, inst, "Instruction forwarded");
    assert_eq!(ex.inst_size, 4, "inst_size forwarded");
    assert_eq!(ex.rd, 1, "rd forwarded");
}

// ══════════════════════════════════════════════════════════
// 13. RV32 flag propagation (W-suffix ops)
// ══════════════════════════════════════════════════════════

#[test]
fn rv32_add_wraps_to_32_bits() {
    let mut tc = ctx();
    let entry = IdExEntry {
        pc: PC,
        inst: InstructionBuilder::new().addw(1, 2, 3).build(),
        inst_size: INST_SIZE,
        rs1: 2,
        rs2: 3,
        rd: 1,
        rv1: 0x1_0000_0000, // bit 32 set
        rv2: 1,
        ctrl: ControlSignals {
            reg_write: true,
            alu: AluOp::Add,
            a_src: OpASrc::Reg1,
            b_src: OpBSrc::Reg2,
            is_rv32: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let ex = exec_one(&mut tc, entry);

    // ADDW: lower 32 bits of (0x1_0000_0000 + 1) = 0x0000_0001, sign-extended
    assert_eq!(ex.alu, 1, "ADDW wraps to 32 bits and sign-extends");
}

#[test]
fn rv32_sub_negative_result_sign_extends() {
    let mut tc = ctx();
    let entry = IdExEntry {
        pc: PC,
        inst: InstructionBuilder::new().subw(1, 2, 3).build(),
        inst_size: INST_SIZE,
        rs1: 2,
        rs2: 3,
        rd: 1,
        rv1: 0,
        rv2: 1,
        ctrl: ControlSignals {
            reg_write: true,
            alu: AluOp::Sub,
            a_src: OpASrc::Reg1,
            b_src: OpBSrc::Reg2,
            is_rv32: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let ex = exec_one(&mut tc, entry);

    // SUBW: 0 - 1 = -1 in 32-bit, sign-extended to 64-bit = 0xFFFF_FFFF_FFFF_FFFF
    assert_eq!(ex.alu as i64, -1, "SUBW negative sign-extends to 64 bits");
}
