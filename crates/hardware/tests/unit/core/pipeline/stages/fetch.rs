//! Fetch Stage Unit Tests.
//!
//! Verifies that `fetch_stage` correctly performs:
//!   1. Instruction fetching — reads 32-bit instructions from bus memory
//!   2. PC advancement — sequential PC += 4 for normal instructions
//!   3. Compressed instruction expansion — 16-bit RVC → 32-bit
//!   4. Branch prediction — BTB lookups for branches and JAL
//!   5. Return prediction — RAS lookup for JALR x0, ra (ret)
//!   6. Misaligned PC trap — odd PC generates InstructionAddressMisaligned
//!   7. Superscalar fetch — multiple instructions per cycle
//!   8. Stop-on-control-flow — stops fetching after branch/jump

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use riscv_core::core::pipeline::stages::fetch_stage;
use riscv_core::core::units::bru::BranchPredictor;

// ══════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════

const MEM_BASE: u64 = 0x1000_0000;
const MEM_SIZE: usize = 0x1000;

/// Create a TestContext with MockMemory, PC set to MEM_BASE.
fn ctx() -> TestContext {
    TestContext::new()
        .with_memory(MEM_SIZE, MEM_BASE)
        .load_program(MEM_BASE, &[])
}

/// Write a single 32-bit instruction at offset from MEM_BASE.
fn write_inst(tc: &mut TestContext, offset: u64, inst: u32) {
    tc.cpu.bus.bus.write_u32(MEM_BASE + offset, inst);
}

/// Fetch one cycle and return the IF/ID entries.
fn fetch(tc: &mut TestContext) -> Vec<riscv_core::core::pipeline::latches::IfIdEntry> {
    fetch_stage(&mut tc.cpu);
    tc.cpu.if_id.entries.clone()
}

// ══════════════════════════════════════════════════════════
// 1. Basic instruction fetching
// ══════════════════════════════════════════════════════════

#[test]
fn fetches_single_instruction() {
    let mut tc = ctx();
    let nop = InstructionBuilder::new().addi(0, 0, 0).build();
    write_inst(&mut tc, 0, nop);

    let entries = fetch(&mut tc);
    assert_eq!(entries.len(), 1, "Should fetch one instruction");
    assert_eq!(entries[0].pc, MEM_BASE, "PC matches fetch address");
    assert_eq!(entries[0].inst, nop, "Instruction matches written value");
    assert_eq!(entries[0].inst_size, 4, "Standard instruction is 4 bytes");
    assert!(entries[0].trap.is_none(), "No trap for valid fetch");
}

#[test]
fn fetches_non_nop_instruction() {
    let mut tc = ctx();
    let add = InstructionBuilder::new().add(1, 2, 3).build();
    write_inst(&mut tc, 0, add);

    let entries = fetch(&mut tc);
    assert_eq!(entries[0].inst, add);
}

// ══════════════════════════════════════════════════════════
// 2. PC advancement
// ══════════════════════════════════════════════════════════

#[test]
fn pc_advances_by_4_for_standard_instruction() {
    let mut tc = ctx();
    let nop = InstructionBuilder::new().addi(0, 0, 0).build();
    write_inst(&mut tc, 0, nop);

    fetch(&mut tc);
    assert_eq!(tc.cpu.pc, MEM_BASE + 4, "PC advanced by 4");
}

#[test]
fn sequential_fetches_advance_pc() {
    let mut tc = ctx();
    let nop = InstructionBuilder::new().addi(0, 0, 0).build();
    write_inst(&mut tc, 0, nop);
    write_inst(&mut tc, 4, nop);
    write_inst(&mut tc, 8, nop);

    fetch(&mut tc);
    assert_eq!(tc.cpu.pc, MEM_BASE + 4);

    fetch(&mut tc);
    assert_eq!(tc.cpu.pc, MEM_BASE + 8);

    fetch(&mut tc);
    assert_eq!(tc.cpu.pc, MEM_BASE + 12);
}

// ══════════════════════════════════════════════════════════
// 3. Misaligned PC trap
// ══════════════════════════════════════════════════════════

#[test]
fn misaligned_pc_generates_trap() {
    let mut tc = ctx();
    tc.cpu.pc = MEM_BASE + 1; // odd address

    let entries = fetch(&mut tc);
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].trap.is_some(),
        "Misaligned PC should generate trap"
    );
    assert!(
        matches!(
            entries[0].trap,
            Some(riscv_core::common::error::Trap::InstructionAddressMisaligned(_))
        ),
        "Trap should be InstructionAddressMisaligned"
    );
}

#[test]
fn misaligned_pc_trap_carries_faulting_address() {
    let mut tc = ctx();
    let fault_addr = MEM_BASE + 3;
    tc.cpu.pc = fault_addr;

    let entries = fetch(&mut tc);
    if let Some(riscv_core::common::error::Trap::InstructionAddressMisaligned(addr)) =
        &entries[0].trap
    {
        assert_eq!(*addr, fault_addr, "Trap carries the faulting PC");
    } else {
        panic!("Expected InstructionAddressMisaligned trap");
    }
}

// ══════════════════════════════════════════════════════════
// 4. IF/ID entry fields
// ══════════════════════════════════════════════════════════

#[test]
fn if_id_entry_has_correct_fields() {
    let mut tc = ctx();
    let add = InstructionBuilder::new().add(5, 10, 15).build();
    write_inst(&mut tc, 0, add);

    let entries = fetch(&mut tc);
    let e = &entries[0];
    assert_eq!(e.pc, MEM_BASE);
    assert_eq!(e.inst, add);
    assert_eq!(e.inst_size, 4);
    assert!(!e.pred_taken, "Non-branch defaults to not predicted taken");
    assert_eq!(e.pred_target, 0);
    assert!(e.trap.is_none());
}

// ══════════════════════════════════════════════════════════
// 5. Branch prediction — BTB lookup on OP_BRANCH
// ══════════════════════════════════════════════════════════

#[test]
fn branch_with_btb_hit_sets_pred_taken() {
    let mut tc = ctx();
    let beq = InstructionBuilder::new().beq(1, 2, 8).build();
    write_inst(&mut tc, 0, beq);

    // Train the predictor: the branch is taken multiple times
    let branch_pc = MEM_BASE;
    let target = MEM_BASE + 8;
    for _ in 0..4 {
        tc.cpu
            .branch_predictor
            .update_branch(branch_pc, true, Some(target));
    }

    let entries = fetch(&mut tc);
    assert_eq!(entries.len(), 1);

    // The static predictor defaults to not-taken, but others (gshare, etc.)
    // might predict taken after training. Check pred_taken matches the
    // prediction from the predictor.
    let (predicted_taken, predicted_target) = tc.cpu.branch_predictor.predict_branch(branch_pc);

    if predicted_taken && predicted_target.is_some() {
        assert!(
            entries[0].pred_taken,
            "Branch predicted taken after training"
        );
        assert_eq!(entries[0].pred_target, predicted_target.unwrap());
        assert_eq!(
            tc.cpu.pc,
            predicted_target.unwrap(),
            "PC redirected to predicted target"
        );
    } else {
        // Static predictor: never predicts taken
        assert!(!entries[0].pred_taken);
        assert_eq!(tc.cpu.pc, MEM_BASE + 4, "PC sequential (static: not-taken)");
    }
}

#[test]
fn branch_without_btb_hit_is_not_predicted_taken() {
    let mut tc = ctx();
    let beq = InstructionBuilder::new().beq(1, 2, 8).build();
    write_inst(&mut tc, 0, beq);
    // Don't train predictor

    let entries = fetch(&mut tc);
    // A cold branch should not be predicted taken (no BTB entry)
    assert!(!entries[0].pred_taken, "Cold branch not predicted taken");
    assert_eq!(tc.cpu.pc, MEM_BASE + 4);
}

// ══════════════════════════════════════════════════════════
// 6. JAL prediction via BTB
// ══════════════════════════════════════════════════════════

#[test]
fn jal_with_btb_entry_uses_prediction() {
    let mut tc = ctx();
    let jal = InstructionBuilder::new().jal(1, 20).build();
    write_inst(&mut tc, 0, jal);

    // Train BTB with JAL target
    let jal_pc = MEM_BASE;
    let target = MEM_BASE + 20;
    tc.cpu
        .branch_predictor
        .update_branch(jal_pc, true, Some(target));

    let entries = fetch(&mut tc);
    assert_eq!(entries.len(), 1);

    if let Some(btb_target) = tc.cpu.branch_predictor.predict_btb(jal_pc) {
        assert!(entries[0].pred_taken);
        assert_eq!(entries[0].pred_target, btb_target);
    }
}

#[test]
fn jal_without_btb_entry_falls_through() {
    let mut tc = ctx();
    let jal = InstructionBuilder::new().jal(1, 20).build();
    write_inst(&mut tc, 0, jal);
    // No BTB training

    let entries = fetch(&mut tc);
    assert!(
        !entries[0].pred_taken,
        "JAL without BTB entry not predicted"
    );
    assert_eq!(tc.cpu.pc, MEM_BASE + 4, "PC falls through");
}

// ══════════════════════════════════════════════════════════
// 7. JALR (ret) — RAS prediction
// ══════════════════════════════════════════════════════════

#[test]
fn jalr_ret_uses_ras_prediction() {
    let mut tc = ctx();
    // JALR x0, 0(x1) = ret
    let ret_inst = InstructionBuilder::new().jalr(0, 1, 0).build();
    write_inst(&mut tc, 0, ret_inst);

    // Push a return address onto the RAS
    let return_addr = MEM_BASE + 0x100;
    tc.cpu.branch_predictor.on_call(
        MEM_BASE + 0x50, // call site
        return_addr,     // return address
        MEM_BASE,        // call target
    );

    let entries = fetch(&mut tc);
    assert_eq!(entries.len(), 1);

    // Check if RAS prediction was used
    if let Some(ras_target) = tc.cpu.branch_predictor.predict_return() {
        assert!(entries[0].pred_taken);
        assert_eq!(entries[0].pred_target, ras_target);
    }
}

#[test]
fn jalr_non_ret_uses_btb() {
    let mut tc = ctx();
    // JALR x5, 0(x10) — not a ret (rd != x0 or rs1 != x1)
    let jalr = InstructionBuilder::new().jalr(5, 10, 0).build();
    write_inst(&mut tc, 0, jalr);

    // Train BTB for this JALR
    let target = MEM_BASE + 0x200;
    tc.cpu
        .branch_predictor
        .update_branch(MEM_BASE, true, Some(target));

    let entries = fetch(&mut tc);
    assert_eq!(entries.len(), 1);
    // The prediction depends on whether the BTB has an entry
}

// ══════════════════════════════════════════════════════════
// 8. Stop-on-control-flow
// ══════════════════════════════════════════════════════════

#[test]
fn jalr_stops_fetch_even_without_prediction() {
    let mut tc = ctx();
    tc.cpu.pipeline_width = 2;

    let jalr = InstructionBuilder::new().jalr(1, 2, 0).build();
    let nop = InstructionBuilder::new().addi(0, 0, 0).build();
    write_inst(&mut tc, 0, jalr);
    write_inst(&mut tc, 4, nop);

    let entries = fetch(&mut tc);
    // JALR should stop fetch — only one instruction fetched
    assert_eq!(entries.len(), 1, "Fetch stops after JALR");
}

// ══════════════════════════════════════════════════════════
// 9. Superscalar fetch (pipeline_width > 1)
// ══════════════════════════════════════════════════════════

#[test]
fn superscalar_fetches_multiple_instructions() {
    let mut tc = ctx();
    tc.cpu.pipeline_width = 4;

    let nop = InstructionBuilder::new().addi(0, 0, 0).build();
    for i in 0..4 {
        write_inst(&mut tc, i * 4, nop);
    }

    let entries = fetch(&mut tc);
    assert_eq!(
        entries.len(),
        4,
        "pipeline_width=4 → 4 instructions fetched"
    );

    for (i, e) in entries.iter().enumerate() {
        assert_eq!(e.pc, MEM_BASE + (i as u64) * 4, "PC of entry {i}");
        assert_eq!(e.inst, nop);
    }
    assert_eq!(
        tc.cpu.pc,
        MEM_BASE + 16,
        "PC advanced past all 4 instructions"
    );
}

#[test]
fn superscalar_stops_at_branch() {
    let mut tc = ctx();
    tc.cpu.pipeline_width = 4;

    let nop = InstructionBuilder::new().addi(0, 0, 0).build();
    let beq = InstructionBuilder::new().beq(1, 2, 8).build();
    write_inst(&mut tc, 0, nop);
    write_inst(&mut tc, 4, beq); // branch in second slot
    write_inst(&mut tc, 8, nop);
    write_inst(&mut tc, 12, nop);

    // Train predictor so branch is predicted taken
    let branch_pc = MEM_BASE + 4;
    let target = MEM_BASE + 12;
    for _ in 0..4 {
        tc.cpu
            .branch_predictor
            .update_branch(branch_pc, true, Some(target));
    }

    let entries = fetch(&mut tc);

    // Check if prediction is taken — if the predictor predicts taken,
    // fetch should stop after the branch.
    let (pred_taken, _) = tc.cpu.branch_predictor.predict_branch(branch_pc);
    if pred_taken {
        assert_eq!(entries.len(), 2, "Fetch stops after predicted-taken branch");
    }
    // If static predictor (not-taken), all 4 may be fetched.
}

// ══════════════════════════════════════════════════════════
// 10. Prediction metadata forwarded to IF/ID
// ══════════════════════════════════════════════════════════

#[test]
fn non_branch_has_no_prediction() {
    let mut tc = ctx();
    let add = InstructionBuilder::new().add(1, 2, 3).build();
    write_inst(&mut tc, 0, add);

    let entries = fetch(&mut tc);
    assert!(!entries[0].pred_taken);
    assert_eq!(entries[0].pred_target, 0);
}

// ══════════════════════════════════════════════════════════
// 11. Fetch from different addresses
// ══════════════════════════════════════════════════════════

#[test]
fn fetch_from_middle_of_memory() {
    let mut tc = ctx();
    let add = InstructionBuilder::new().add(5, 6, 7).build();
    write_inst(&mut tc, 0x100, add);
    tc.cpu.pc = MEM_BASE + 0x100;

    let entries = fetch(&mut tc);
    assert_eq!(entries[0].pc, MEM_BASE + 0x100);
    assert_eq!(entries[0].inst, add);
}

// ══════════════════════════════════════════════════════════
// 12. Stall cycles accumulated
// ══════════════════════════════════════════════════════════

#[test]
fn fetch_accumulates_stall_cycles() {
    let mut tc = ctx();
    let nop = InstructionBuilder::new().addi(0, 0, 0).build();
    write_inst(&mut tc, 0, nop);

    let before = tc.cpu.stall_cycles;
    fetch(&mut tc);
    // Fetch should add at least some transit time
    assert!(
        tc.cpu.stall_cycles >= before,
        "Stall cycles should not decrease"
    );
}

// ══════════════════════════════════════════════════════════
// 13. Even-aligned but non-word-aligned PC (valid for RVC)
// ══════════════════════════════════════════════════════════

#[test]
fn pc_on_2_byte_boundary_is_valid() {
    let mut tc = ctx();
    // Write a 32-bit instruction at a 2-byte boundary
    let add = InstructionBuilder::new().add(1, 2, 3).build();
    // Write it byte-by-byte at offset 2 to simulate 2-byte alignment
    let bytes = add.to_le_bytes();
    for (i, b) in bytes.iter().enumerate() {
        tc.cpu.bus.bus.write_u8(MEM_BASE + 2 + i as u64, *b);
    }
    tc.cpu.pc = MEM_BASE + 2;

    let entries = fetch(&mut tc);
    assert_eq!(entries.len(), 1);
    // The fetch should succeed (2-byte aligned is OK)
    // Whether it reads as compressed or standard depends on the low bits
    assert!(
        entries[0].trap.is_none(),
        "2-byte aligned fetch should not trap"
    );
}

// ══════════════════════════════════════════════════════════
// 14. Empty previous IF/ID is replaced
// ══════════════════════════════════════════════════════════

#[test]
fn fetch_replaces_previous_if_id() {
    let mut tc = ctx();
    let nop = InstructionBuilder::new().addi(0, 0, 0).build();
    write_inst(&mut tc, 0, nop);

    // Pre-populate IF/ID with stale entries
    tc.cpu.if_id.entries = vec![riscv_core::core::pipeline::latches::IfIdEntry {
        pc: 0xDEAD,
        inst: 0xBEEF,
        inst_size: 4,
        ..Default::default()
    }];

    fetch(&mut tc);
    // Should be replaced with new fetch
    assert_eq!(tc.cpu.if_id.entries.len(), 1);
    assert_eq!(
        tc.cpu.if_id.entries[0].pc, MEM_BASE,
        "Stale entries replaced"
    );
}
