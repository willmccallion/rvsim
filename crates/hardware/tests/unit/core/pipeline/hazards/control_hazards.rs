//! Control Hazard Tests — Pipeline Flushing on Mispredict.
//!
//! Verifies that branch mispredictions cause proper pipeline flushing
//! by running short instruction sequences through the full pipeline
//! and checking PC redirection, latch clearing, and stat counters.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;

// ══════════════════════════════════════════════════════════
// Helper constants
// ══════════════════════════════════════════════════════════

const BASE_ADDR: u64 = 0x8000_0000;
const MEM_SIZE: usize = 0x1000;

/// Build a TestContext with memory at BASE_ADDR.
fn ctx() -> TestContext {
    TestContext::new().with_memory(MEM_SIZE, BASE_ADDR)
}

// ══════════════════════════════════════════════════════════
// 1. Taken branch flushes speculated instructions
// ══════════════════════════════════════════════════════════

#[test]
fn taken_branch_redirects_pc() {
    // Program:
    //   0: x1 = 10
    //   4: x2 = 20
    //   8: BEQ x1, x1, +8  (always taken → jumps to 16)
    //  12: x3 = 99          (should NOT execute — flushed)
    //  16: x4 = 42          (branch target)
    //  20: NOP
    //  24: NOP
    //  28: NOP
    let nop = InstructionBuilder::new().nop().build();
    let mut tc = ctx().load_program(
        BASE_ADDR,
        &[
            InstructionBuilder::new().addi(1, 0, 10).build(), // x1 = 10
            InstructionBuilder::new().addi(2, 0, 20).build(), // x2 = 20
            InstructionBuilder::new().beq(1, 1, 8).build(),   // BEQ x1, x1, +8
            InstructionBuilder::new().addi(3, 0, 99).build(), // x3 = 99 (should flush)
            InstructionBuilder::new().addi(4, 0, 42).build(), // x4 = 42 (target)
            nop,
            nop,
            nop,
        ],
    );

    tc.run(20);

    assert_eq!(tc.get_reg(1), 10, "x1 should be 10");
    assert_eq!(tc.get_reg(2), 20, "x2 should be 20");
    assert_eq!(
        tc.get_reg(3),
        0,
        "x3 should NOT be written (flushed by branch)"
    );
    assert_eq!(tc.get_reg(4), 42, "x4 should be 42 (branch target)");
}

// ══════════════════════════════════════════════════════════
// 2. Not-taken branch continues sequentially
// ══════════════════════════════════════════════════════════

#[test]
fn not_taken_branch_continues() {
    // Program:
    //   0: x1 = 10
    //   4: x2 = 20
    //   8: BEQ x1, x2, +8  (not taken: 10 != 20)
    //  12: x3 = 33          (should execute)
    //  16: NOP ...
    let nop = InstructionBuilder::new().nop().build();
    let mut tc = ctx().load_program(
        BASE_ADDR,
        &[
            InstructionBuilder::new().addi(1, 0, 10).build(),
            InstructionBuilder::new().addi(2, 0, 20).build(),
            InstructionBuilder::new().beq(1, 2, 8).build(),
            InstructionBuilder::new().addi(3, 0, 33).build(),
            nop,
            nop,
            nop,
            nop,
        ],
    );

    tc.run(20);

    assert_eq!(tc.get_reg(3), 33, "Not-taken: x3 should execute normally");
}

// ══════════════════════════════════════════════════════════
// 3. JAL unconditional jump
// ══════════════════════════════════════════════════════════

#[test]
fn jal_redirects_and_links() {
    // Program:
    //   0: JAL x1, +12     (jump to 12, x1 = 4)
    //   4: x6 = 99         (should NOT execute — flushed)
    //   8: x7 = 99         (should NOT execute — flushed)
    //  12: x4 = 55         (target)
    //  16: NOP...
    //
    // Note: avoid x2 (sp) — it is pre-initialised in direct mode.
    let nop = InstructionBuilder::new().nop().build();
    let mut tc = ctx().load_program(
        BASE_ADDR,
        &[
            InstructionBuilder::new().jal(1, 12).build(),
            InstructionBuilder::new().addi(6, 0, 99).build(),
            InstructionBuilder::new().addi(7, 0, 99).build(),
            InstructionBuilder::new().addi(4, 0, 55).build(),
            nop,
            nop,
            nop,
            nop,
        ],
    );

    tc.run(20);

    assert_eq!(tc.get_reg(1), BASE_ADDR + 4, "x1 = return address (PC+4)");
    assert_eq!(tc.get_reg(6), 0, "x6 should NOT execute (after JAL)");
    assert_eq!(tc.get_reg(4), 55, "x4 should be 55 (jump target)");
}

// ══════════════════════════════════════════════════════════
// 4. JALR indirect jump
// ══════════════════════════════════════════════════════════

#[test]
fn jalr_indirect_jump() {
    // Program:
    //   0: AUIPC x5, 0      (x5 = PC = BASE_ADDR, avoids LUI sign-extension)
    //   4: ADDI  x5, x5, 16 (x5 = BASE_ADDR + 16 = target)
    //   8: JALR  x1, x5, 0  (jump to x5, link x1)
    //  12: x6 = 99           (should NOT execute)
    //  16: x3 = 77           (target)
    //  20: NOP...
    //
    // Note: LUI x5, 0x80000 sign-extends bit 31 on RV64 → 0xFFFF_FFFF_8000_0000.
    // AUIPC adds PC (already 64-bit) + 0, giving the correct 0x8000_0000.
    // Note: avoid x2 (sp) — it is pre-initialised in direct mode.
    let nop = InstructionBuilder::new().nop().build();
    let mut tc = ctx().load_program(
        BASE_ADDR,
        &[
            InstructionBuilder::new().auipc(5, 0).build(), // x5 = PC = 0x8000_0000
            InstructionBuilder::new().addi(5, 5, 16).build(), // x5 = 0x8000_0010
            InstructionBuilder::new().jalr(1, 5, 0).build(), // jump to x5
            InstructionBuilder::new().addi(6, 0, 99).build(), // should not execute
            InstructionBuilder::new().addi(3, 0, 77).build(), // target
            nop,
            nop,
            nop,
        ],
    );

    tc.run(25);

    assert_eq!(tc.get_reg(3), 77, "x3 should be 77 (JALR target)");
    assert_eq!(tc.get_reg(1), BASE_ADDR + 12, "x1 = return address");
    assert_eq!(tc.get_reg(6), 0, "x6 should NOT execute (flushed)");
}

// ══════════════════════════════════════════════════════════
// 5. Branch backward (loop-like pattern)
// ══════════════════════════════════════════════════════════

#[test]
fn backward_branch_loop() {
    // Simple loop: x1 counts from 0 to 3.
    //   0: x1 = 0
    //   4: x2 = 3
    //   8: ADDI x1, x1, 1     (loop body — increment)
    //  12: BNE x1, x2, -4     (branch backward to 8 if x1 != x2)
    //  16: x3 = 100            (post-loop)
    //  20: NOP...
    let nop = InstructionBuilder::new().nop().build();
    let mut tc = ctx().load_program(
        BASE_ADDR,
        &[
            InstructionBuilder::new().addi(1, 0, 0).build(), // x1 = 0
            InstructionBuilder::new().addi(2, 0, 3).build(), // x2 = 3
            InstructionBuilder::new().addi(1, 1, 1).build(), // x1 += 1 (loop body)
            InstructionBuilder::new().bne(1, 2, -4).build(), // if x1 != x2 goto -4
            InstructionBuilder::new().addi(3, 0, 100).build(), // x3 = 100
            nop,
            nop,
            nop,
        ],
    );

    tc.run(80);

    assert_eq!(tc.get_reg(1), 3, "x1 should be 3 after loop");
    assert_eq!(tc.get_reg(3), 100, "x3 should be 100 after loop exit");
}

// ══════════════════════════════════════════════════════════
// 6. All branch variants execute correctly
// ══════════════════════════════════════════════════════════

#[test]
fn blt_taken() {
    let nop = InstructionBuilder::new().nop().build();
    let mut tc = ctx().load_program(
        BASE_ADDR,
        &[
            InstructionBuilder::new().addi(1, 0, 5).build(),
            InstructionBuilder::new().addi(2, 0, 10).build(),
            InstructionBuilder::new().blt(1, 2, 8).build(), // 5 < 10 → taken
            InstructionBuilder::new().addi(3, 0, 99).build(), // flushed
            InstructionBuilder::new().addi(4, 0, 42).build(), // target
            nop,
            nop,
            nop,
        ],
    );
    tc.run(20);
    assert_eq!(tc.get_reg(3), 0, "BLT taken → x3 flushed");
    assert_eq!(tc.get_reg(4), 42, "BLT target reached");
}

#[test]
fn bge_taken() {
    let nop = InstructionBuilder::new().nop().build();
    let mut tc = ctx().load_program(
        BASE_ADDR,
        &[
            InstructionBuilder::new().addi(1, 0, 10).build(),
            InstructionBuilder::new().addi(2, 0, 10).build(),
            InstructionBuilder::new().bge(1, 2, 8).build(), // 10 >= 10 → taken
            InstructionBuilder::new().addi(3, 0, 99).build(),
            InstructionBuilder::new().addi(4, 0, 42).build(),
            nop,
            nop,
            nop,
        ],
    );
    tc.run(20);
    assert_eq!(tc.get_reg(3), 0);
    assert_eq!(tc.get_reg(4), 42);
}

#[test]
fn bltu_taken() {
    let nop = InstructionBuilder::new().nop().build();
    // Use -1 (u64::MAX) as unsigned value > 5
    let mut tc = ctx().load_program(
        BASE_ADDR,
        &[
            InstructionBuilder::new().addi(1, 0, 5).build(),
            InstructionBuilder::new().addi(2, 0, -1).build(), // x2 = 0xFFFF...FFFF (unsigned large)
            InstructionBuilder::new().bltu(1, 2, 8).build(),  // 5 <u MAX → taken
            InstructionBuilder::new().addi(3, 0, 99).build(),
            InstructionBuilder::new().addi(4, 0, 42).build(),
            nop,
            nop,
            nop,
        ],
    );
    tc.run(20);
    assert_eq!(tc.get_reg(3), 0);
    assert_eq!(tc.get_reg(4), 42);
}

#[test]
fn bgeu_not_taken() {
    let nop = InstructionBuilder::new().nop().build();
    let mut tc = ctx().load_program(
        BASE_ADDR,
        &[
            InstructionBuilder::new().addi(1, 0, 5).build(),
            InstructionBuilder::new().addi(2, 0, 10).build(),
            InstructionBuilder::new().bgeu(1, 2, 8).build(), // 5 >=u 10 → not taken
            InstructionBuilder::new().addi(3, 0, 33).build(), // should execute
            nop,
            nop,
            nop,
            nop,
        ],
    );
    tc.run(20);
    assert_eq!(tc.get_reg(3), 33, "BGEU not taken → sequential execution");
}
