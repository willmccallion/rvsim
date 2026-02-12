//! Writeback Stage Unit Tests.
//!
//! Verifies that `wb_stage` correctly performs:
//!   1. Register writeback — ALU result, load data, jump link (PC+4)
//!   2. x0 writes discarded — no value stored to x0
//!   3. FP register writes — value written to FPR
//!   4. Instruction retirement stats — counters for alu, load, store, branch, system, fp
//!   5. Trap handling — pipeline flushed, cpu.trap() called
//!   6. PC trace updated — (pc, inst) pushed to trace buffer
//!   7. Multiple entries all retire
//!   8. NOP / zero instruction not counted

use crate::common::harness::TestContext;
use riscv_core::core::pipeline::latches::MemWbEntry;
use riscv_core::core::pipeline::signals::{AluOp, ControlSignals, MemWidth};
use riscv_core::core::pipeline::stages::wb_stage;

// ══════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════

const PC: u64 = 0x8000_0000;
const INST_SIZE: u64 = 4;

fn ctx() -> TestContext {
    TestContext::new()
}

/// Build a MemWbEntry for an ALU instruction that writes to rd.
fn alu_wb(rd: usize, alu: u64) -> MemWbEntry {
    MemWbEntry {
        pc: PC,
        inst: 0x0033_0033, // non-NOP encoding
        inst_size: INST_SIZE,
        rd,
        alu,
        load_data: 0,
        ctrl: ControlSignals {
            reg_write: true,
            alu: AluOp::Add,
            ..Default::default()
        },
        trap: None,
    }
}

/// Build a MemWbEntry for a load instruction.
fn load_wb(rd: usize, load_data: u64) -> MemWbEntry {
    MemWbEntry {
        pc: PC,
        inst: 0x0000_0003,
        inst_size: INST_SIZE,
        rd,
        alu: 0x1000, // load address (not used for writeback value)
        load_data,
        ctrl: ControlSignals {
            reg_write: true,
            mem_read: true,
            width: MemWidth::Word,
            ..Default::default()
        },
        trap: None,
    }
}

/// Build a MemWbEntry for a jump instruction (link register).
fn jump_wb(rd: usize, pc: u64) -> MemWbEntry {
    MemWbEntry {
        pc,
        inst: 0x0000_006F, // JAL-like encoding
        inst_size: INST_SIZE,
        rd,
        alu: 0,
        load_data: 0,
        ctrl: ControlSignals {
            reg_write: true,
            jump: true,
            ..Default::default()
        },
        trap: None,
    }
}

/// Build a MemWbEntry for a store instruction (no register writeback).
fn store_wb() -> MemWbEntry {
    MemWbEntry {
        pc: PC,
        inst: 0x0000_0023,
        inst_size: INST_SIZE,
        rd: 0,
        alu: 0x1000,
        load_data: 0,
        ctrl: ControlSignals {
            mem_write: true,
            width: MemWidth::Word,
            ..Default::default()
        },
        trap: None,
    }
}

/// Build a MemWbEntry for a branch instruction.
fn branch_wb() -> MemWbEntry {
    MemWbEntry {
        pc: PC,
        inst: 0x0000_0063,
        inst_size: INST_SIZE,
        ctrl: ControlSignals {
            branch: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Build a MemWbEntry for a system instruction.
fn system_wb() -> MemWbEntry {
    MemWbEntry {
        pc: PC,
        inst: 0x0000_0073,
        inst_size: INST_SIZE,
        ctrl: ControlSignals {
            is_system: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Run wb_stage with a single MemWbEntry.
fn wb_one(tc: &mut TestContext, entry: MemWbEntry) {
    tc.cpu.mem_wb.entries = vec![entry];
    wb_stage(&mut tc.cpu);
}

// ══════════════════════════════════════════════════════════
// 1. Register writeback — ALU result
// ══════════════════════════════════════════════════════════

#[test]
fn alu_result_written_to_rd() {
    let mut tc = ctx();
    wb_one(&mut tc, alu_wb(5, 0xCAFE));
    assert_eq!(tc.cpu.regs.read(5), 0xCAFE, "ALU result written to x5");
}

#[test]
fn alu_result_written_to_different_regs() {
    let mut tc = ctx();
    wb_one(&mut tc, alu_wb(10, 100));
    assert_eq!(tc.cpu.regs.read(10), 100);

    wb_one(&mut tc, alu_wb(31, 999));
    assert_eq!(tc.cpu.regs.read(31), 999);
}

// ══════════════════════════════════════════════════════════
// 2. x0 writes discarded
// ══════════════════════════════════════════════════════════

#[test]
fn write_to_x0_discarded() {
    let mut tc = ctx();
    wb_one(&mut tc, alu_wb(0, 0xDEAD));
    assert_eq!(tc.cpu.regs.read(0), 0, "x0 must always be 0");
}

// ══════════════════════════════════════════════════════════
// 3. Load data written for mem_read instructions
// ══════════════════════════════════════════════════════════

#[test]
fn load_data_written_to_rd() {
    let mut tc = ctx();
    wb_one(&mut tc, load_wb(7, 0xDEAD_BEEF));
    assert_eq!(
        tc.cpu.regs.read(7),
        0xDEAD_BEEF,
        "Load data written to register"
    );
}

#[test]
fn load_data_preferred_over_alu() {
    let mut tc = ctx();
    let entry = MemWbEntry {
        rd: 3,
        alu: 0x1111, // address, not the value to write
        load_data: 0x2222,
        ctrl: ControlSignals {
            reg_write: true,
            mem_read: true,
            ..Default::default()
        },
        inst: 0x0000_0003,
        inst_size: INST_SIZE,
        pc: PC,
        trap: None,
    };
    wb_one(&mut tc, entry);
    assert_eq!(
        tc.cpu.regs.read(3),
        0x2222,
        "mem_read: load_data takes priority over alu"
    );
}

// ══════════════════════════════════════════════════════════
// 4. Jump link — PC + inst_size written to rd
// ══════════════════════════════════════════════════════════

#[test]
fn jump_writes_link_address() {
    let mut tc = ctx();
    wb_one(&mut tc, jump_wb(1, 0x8000_0100));
    assert_eq!(
        tc.cpu.regs.read(1),
        0x8000_0100 + INST_SIZE,
        "JAL/JALR: rd = PC + 4 (link address)"
    );
}

#[test]
fn jump_x0_discards_link() {
    let mut tc = ctx();
    wb_one(&mut tc, jump_wb(0, 0x8000_0100));
    assert_eq!(tc.cpu.regs.read(0), 0, "Jump to x0 discards link value");
}

// ══════════════════════════════════════════════════════════
// 5. FP register writes
// ══════════════════════════════════════════════════════════

#[test]
fn fp_result_written_to_fpr() {
    let mut tc = ctx();
    let entry = MemWbEntry {
        pc: PC,
        inst: 0x5300_0053, // non-NOP placeholder for FP
        inst_size: INST_SIZE,
        rd: 3,
        alu: 0x4000_0000_0000_0000, // 2.0 in f64
        ctrl: ControlSignals {
            fp_reg_write: true,
            alu: AluOp::FAdd,
            ..Default::default()
        },
        ..Default::default()
    };
    wb_one(&mut tc, entry);
    assert_eq!(
        tc.cpu.regs.read_f(3),
        0x4000_0000_0000_0000,
        "FP result written to f3"
    );
}

#[test]
fn fp_load_written_to_fpr() {
    let mut tc = ctx();
    let entry = MemWbEntry {
        pc: PC,
        inst: 0x0000_2007, // placeholder FLW
        inst_size: INST_SIZE,
        rd: 5,
        load_data: 0xFFFF_FFFF_3F80_0000, // NaN-boxed 1.0f
        ctrl: ControlSignals {
            fp_reg_write: true,
            mem_read: true,
            width: MemWidth::Word,
            ..Default::default()
        },
        ..Default::default()
    };
    wb_one(&mut tc, entry);
    assert_eq!(
        tc.cpu.regs.read_f(5),
        0xFFFF_FFFF_3F80_0000,
        "FP load data written to f5"
    );
}

// ══════════════════════════════════════════════════════════
// 6. Instruction retirement stats
// ══════════════════════════════════════════════════════════

#[test]
fn alu_instruction_increments_inst_alu() {
    let mut tc = ctx();
    let before = tc.cpu.stats.inst_alu;
    wb_one(&mut tc, alu_wb(1, 42));
    assert_eq!(tc.cpu.stats.inst_alu, before + 1);
}

#[test]
fn load_instruction_increments_inst_load() {
    let mut tc = ctx();
    let before = tc.cpu.stats.inst_load;
    wb_one(&mut tc, load_wb(1, 42));
    assert_eq!(tc.cpu.stats.inst_load, before + 1);
}

#[test]
fn store_instruction_increments_inst_store() {
    let mut tc = ctx();
    let before = tc.cpu.stats.inst_store;
    wb_one(&mut tc, store_wb());
    assert_eq!(tc.cpu.stats.inst_store, before + 1);
}

#[test]
fn branch_instruction_increments_inst_branch() {
    let mut tc = ctx();
    let before = tc.cpu.stats.inst_branch;
    wb_one(&mut tc, branch_wb());
    assert_eq!(tc.cpu.stats.inst_branch, before + 1);
}

#[test]
fn system_instruction_increments_inst_system() {
    let mut tc = ctx();
    let before = tc.cpu.stats.inst_system;
    wb_one(&mut tc, system_wb());
    assert_eq!(tc.cpu.stats.inst_system, before + 1);
}

#[test]
fn instructions_retired_counter_increments() {
    let mut tc = ctx();
    let before = tc.cpu.stats.instructions_retired;
    wb_one(&mut tc, alu_wb(1, 42));
    assert_eq!(tc.cpu.stats.instructions_retired, before + 1);
}

#[test]
fn fp_arith_increments_inst_fp_arith() {
    let mut tc = ctx();
    let before = tc.cpu.stats.inst_fp_arith;
    let entry = MemWbEntry {
        pc: PC,
        inst: 0x5300_0053,
        inst_size: INST_SIZE,
        rd: 1,
        alu: 0,
        ctrl: ControlSignals {
            fp_reg_write: true,
            alu: AluOp::FAdd,
            ..Default::default()
        },
        ..Default::default()
    };
    wb_one(&mut tc, entry);
    assert_eq!(tc.cpu.stats.inst_fp_arith, before + 1);
}

// ══════════════════════════════════════════════════════════
// 7. NOP / zero instruction not counted
// ══════════════════════════════════════════════════════════

#[test]
fn nop_not_counted() {
    let mut tc = ctx();
    let before = tc.cpu.stats.instructions_retired;
    let entry = MemWbEntry {
        pc: PC,
        inst: 0x13, // NOP (ADDI x0, x0, 0)
        inst_size: INST_SIZE,
        ..Default::default()
    };
    wb_one(&mut tc, entry);
    assert_eq!(
        tc.cpu.stats.instructions_retired, before,
        "NOP (0x13) should not be counted"
    );
}

#[test]
fn zero_inst_not_counted() {
    let mut tc = ctx();
    let before = tc.cpu.stats.instructions_retired;
    let entry = MemWbEntry {
        pc: PC,
        inst: 0x0,
        inst_size: INST_SIZE,
        ..Default::default()
    };
    wb_one(&mut tc, entry);
    assert_eq!(
        tc.cpu.stats.instructions_retired, before,
        "Zero instruction should not be counted"
    );
}

// ══════════════════════════════════════════════════════════
// 8. PC trace updated
// ══════════════════════════════════════════════════════════

#[test]
fn pc_trace_updated_on_retire() {
    let mut tc = ctx();
    let inst = 0x0033_0033_u32;
    let before_len = tc.cpu.pc_trace.len();
    wb_one(&mut tc, alu_wb(1, 42));
    assert_eq!(tc.cpu.pc_trace.len(), before_len + 1);
    let (trace_pc, trace_inst) = tc.cpu.pc_trace.last().unwrap();
    assert_eq!(*trace_pc, PC);
    assert_eq!(*trace_inst, inst);
}

// ══════════════════════════════════════════════════════════
// 9. Trap handling
// ══════════════════════════════════════════════════════════

#[test]
fn trap_flushes_pipeline_latches() {
    let mut tc = ctx();
    // In direct mode, IllegalInstruction causes exit_code=1
    let trap = riscv_core::common::error::Trap::IllegalInstruction(0xDEAD);
    let entry = MemWbEntry {
        pc: PC,
        inst: 0xDEAD,
        inst_size: INST_SIZE,
        trap: Some(trap),
        ..Default::default()
    };
    wb_one(&mut tc, entry);

    // Pipeline latches should be flushed
    assert!(tc.cpu.if_id.entries.is_empty(), "IF/ID flushed on trap");
    assert!(tc.cpu.id_ex.entries.is_empty(), "ID/EX flushed on trap");
    assert!(tc.cpu.ex_mem.entries.is_empty(), "EX/MEM flushed on trap");
}

#[test]
fn trap_sets_exit_code_in_direct_mode() {
    let mut tc = ctx();
    let trap = riscv_core::common::error::Trap::IllegalInstruction(0xDEAD);
    let entry = MemWbEntry {
        pc: PC,
        inst: 0xDEAD,
        inst_size: INST_SIZE,
        trap: Some(trap),
        ..Default::default()
    };
    wb_one(&mut tc, entry);
    assert!(
        tc.cpu.exit_code.is_some(),
        "Direct mode trap sets exit_code"
    );
}

#[test]
fn trap_stops_processing_subsequent_entries() {
    let mut tc = ctx();
    let trap = riscv_core::common::error::Trap::IllegalInstruction(0xDEAD);
    let trap_entry = MemWbEntry {
        pc: PC,
        inst: 0xDEAD,
        inst_size: INST_SIZE,
        trap: Some(trap),
        ..Default::default()
    };
    let normal_entry = alu_wb(10, 0xBEEF);

    tc.cpu.mem_wb.entries = vec![trap_entry, normal_entry];
    wb_stage(&mut tc.cpu);

    assert_eq!(
        tc.cpu.regs.read(10),
        0,
        "Entry after trap should not be retired"
    );
}

#[test]
fn normal_entry_before_trap_still_retires() {
    let mut tc = ctx();
    let normal = alu_wb(8, 0xCAFE);
    let trap = riscv_core::common::error::Trap::IllegalInstruction(0xDEAD);
    let trap_entry = MemWbEntry {
        pc: PC,
        inst: 0xDEAD,
        inst_size: INST_SIZE,
        trap: Some(trap),
        ..Default::default()
    };

    tc.cpu.mem_wb.entries = vec![normal, trap_entry];
    wb_stage(&mut tc.cpu);

    // The first entry should have been committed before the trap was processed.
    // However, in direct mode the trap handler sets exit_code and pipeline is flushed.
    // The key test: the register write from the normal entry should have happened.
    assert_eq!(
        tc.cpu.regs.read(8),
        0xCAFE,
        "Entry before trap should be committed"
    );
}

// ══════════════════════════════════════════════════════════
// 10. Multiple entries all retire
// ══════════════════════════════════════════════════════════

#[test]
fn multiple_entries_all_write_registers() {
    let mut tc = ctx();
    let e1 = alu_wb(5, 100);
    let e2 = alu_wb(6, 200);

    tc.cpu.mem_wb.entries = vec![e1, e2];
    wb_stage(&mut tc.cpu);

    assert_eq!(tc.cpu.regs.read(5), 100);
    assert_eq!(tc.cpu.regs.read(6), 200);
}

#[test]
fn multiple_entries_increment_stats() {
    let mut tc = ctx();
    let before = tc.cpu.stats.instructions_retired;

    let e1 = alu_wb(5, 100);
    let e2 = load_wb(6, 200);

    tc.cpu.mem_wb.entries = vec![e1, e2];
    wb_stage(&mut tc.cpu);

    assert_eq!(tc.cpu.stats.instructions_retired, before + 2);
}

// ══════════════════════════════════════════════════════════
// 11. Empty MEM/WB is a no-op
// ══════════════════════════════════════════════════════════

#[test]
fn empty_mem_wb_is_noop() {
    let mut tc = ctx();
    let before_retired = tc.cpu.stats.instructions_retired;
    tc.cpu.mem_wb.entries.clear();
    wb_stage(&mut tc.cpu);
    assert_eq!(tc.cpu.stats.instructions_retired, before_retired);
}

// ══════════════════════════════════════════════════════════
// 12. Store does not write to register
// ══════════════════════════════════════════════════════════

#[test]
fn store_does_not_write_register() {
    let mut tc = ctx();
    tc.cpu.regs.write(3, 0xAAAA);
    let entry = MemWbEntry {
        pc: PC,
        inst: 0x0000_0023,
        inst_size: INST_SIZE,
        rd: 3, // even if rd is set, no reg_write flag
        alu: 42,
        ctrl: ControlSignals {
            mem_write: true,
            width: MemWidth::Word,
            ..Default::default()
        },
        ..Default::default()
    };
    wb_one(&mut tc, entry);
    assert_eq!(
        tc.cpu.regs.read(3),
        0xAAAA,
        "Store must not modify registers"
    );
}

// ══════════════════════════════════════════════════════════
// 13. Jump stat counted as branch
// ══════════════════════════════════════════════════════════

#[test]
fn jump_increments_inst_branch() {
    let mut tc = ctx();
    let before = tc.cpu.stats.inst_branch;
    wb_one(&mut tc, jump_wb(1, PC));
    assert_eq!(
        tc.cpu.stats.inst_branch,
        before + 1,
        "Jump instruction counted as branch"
    );
}

// ══════════════════════════════════════════════════════════
// 14. FP store counted
// ══════════════════════════════════════════════════════════

#[test]
fn fp_store_increments_inst_fp_store() {
    let mut tc = ctx();
    let before = tc.cpu.stats.inst_fp_store;
    let entry = MemWbEntry {
        pc: PC,
        inst: 0x0000_0027, // placeholder FSW
        inst_size: INST_SIZE,
        ctrl: ControlSignals {
            mem_write: true,
            rs2_fp: true,
            width: MemWidth::Word,
            ..Default::default()
        },
        ..Default::default()
    };
    wb_one(&mut tc, entry);
    assert_eq!(tc.cpu.stats.inst_fp_store, before + 1);
}

#[test]
fn fp_load_increments_inst_fp_load() {
    let mut tc = ctx();
    let before = tc.cpu.stats.inst_fp_load;
    let entry = MemWbEntry {
        pc: PC,
        inst: 0x0000_2007,
        inst_size: INST_SIZE,
        rd: 1,
        load_data: 0xFFFF_FFFF_3F80_0000,
        ctrl: ControlSignals {
            fp_reg_write: true,
            mem_read: true,
            width: MemWidth::Word,
            ..Default::default()
        },
        ..Default::default()
    };
    wb_one(&mut tc, entry);
    assert_eq!(tc.cpu.stats.inst_fp_load, before + 1);
}
