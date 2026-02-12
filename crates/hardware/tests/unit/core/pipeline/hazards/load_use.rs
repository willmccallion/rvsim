//! Load-Use Hazard Detection Tests.
//!
//! Verifies that `need_stall_load_use` correctly detects when a stall
//! is required because an instruction in Decode depends on data being
//! loaded by an instruction in Execute.

use riscv_core::core::pipeline::hazards::need_stall_load_use;
use riscv_core::core::pipeline::latches::{IdEx, IdExEntry, IfId, IfIdEntry};
use riscv_core::core::pipeline::signals::ControlSignals;

/// Helper: encode a minimal instruction with given rs1 and rs2 fields.
fn encode_inst(rs1: u32, rs2: u32) -> u32 {
    // We only need bits 15-19 (rs1) and 20-24 (rs2)
    (rs1 & 0x1F) << 15 | (rs2 & 0x1F) << 20
}

/// Helper: create an IdExEntry that is a load writing to rd.
fn load_entry(rd: usize) -> IdExEntry {
    IdExEntry {
        rd,
        ctrl: ControlSignals {
            mem_read: true,
            reg_write: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Helper: create an IdExEntry that is an ALU write to rd (no load).
fn alu_entry(rd: usize) -> IdExEntry {
    IdExEntry {
        rd,
        ctrl: ControlSignals {
            reg_write: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Helper: create an IfIdEntry with an instruction using given source registers.
fn if_id_entry(rs1: u32, rs2: u32) -> IfIdEntry {
    IfIdEntry {
        inst: encode_inst(rs1, rs2),
        ..Default::default()
    }
}

// ══════════════════════════════════════════════════════════
// 1. Basic load-use detection
// ══════════════════════════════════════════════════════════

#[test]
fn stall_when_load_rd_matches_rs1() {
    let id_ex = IdEx {
        entries: vec![load_entry(5)],
    };
    let if_id = IfId {
        entries: vec![if_id_entry(5, 0)],
    };
    assert!(
        need_stall_load_use(&id_ex, &if_id),
        "Load x5, then use x5 as rs1 → stall"
    );
}

#[test]
fn stall_when_load_rd_matches_rs2() {
    let id_ex = IdEx {
        entries: vec![load_entry(7)],
    };
    let if_id = IfId {
        entries: vec![if_id_entry(0, 7)],
    };
    assert!(
        need_stall_load_use(&id_ex, &if_id),
        "Load x7, then use x7 as rs2 → stall"
    );
}

#[test]
fn stall_when_load_rd_matches_rs3() {
    // rs3 is bits 27-31 of the instruction
    let id_ex = IdEx {
        entries: vec![load_entry(4)],
    };
    let inst = (4u32 & 0x1F) << 27; // rs3=4
    let if_id = IfId {
        entries: vec![IfIdEntry {
            inst,
            ..Default::default()
        }],
    };
    assert!(
        need_stall_load_use(&id_ex, &if_id),
        "Load x4, then use x4 as rs3 → stall"
    );
}

// ══════════════════════════════════════════════════════════
// 2. No stall cases
// ══════════════════════════════════════════════════════════

#[test]
fn no_stall_when_no_load() {
    // ALU instruction (not a load) writing to x5 — no stall needed.
    let id_ex = IdEx {
        entries: vec![alu_entry(5)],
    };
    let if_id = IfId {
        entries: vec![if_id_entry(5, 0)],
    };
    assert!(
        !need_stall_load_use(&id_ex, &if_id),
        "Non-load ALU → no stall"
    );
}

#[test]
fn no_stall_when_no_dependency() {
    let id_ex = IdEx {
        entries: vec![load_entry(5)],
    };
    let if_id = IfId {
        entries: vec![if_id_entry(6, 7)], // uses x6 and x7, not x5
    };
    assert!(
        !need_stall_load_use(&id_ex, &if_id),
        "No register overlap → no stall"
    );
}

#[test]
fn no_stall_when_load_targets_x0() {
    // Load into x0 is effectively a NOP (x0 hardwired to 0).
    let mut entry = load_entry(0);
    entry.ctrl.fp_reg_write = false;
    let id_ex = IdEx {
        entries: vec![entry],
    };
    let if_id = IfId {
        entries: vec![if_id_entry(0, 0)],
    };
    assert!(
        !need_stall_load_use(&id_ex, &if_id),
        "Load to x0 → never stall"
    );
}

#[test]
fn no_stall_when_empty_pipeline() {
    let id_ex = IdEx { entries: vec![] };
    let if_id = IfId { entries: vec![] };
    assert!(!need_stall_load_use(&id_ex, &if_id));
}

#[test]
fn no_stall_when_id_empty() {
    let id_ex = IdEx {
        entries: vec![load_entry(5)],
    };
    let if_id = IfId { entries: vec![] };
    assert!(!need_stall_load_use(&id_ex, &if_id));
}

// ══════════════════════════════════════════════════════════
// 3. FP load-use detection
// ══════════════════════════════════════════════════════════

#[test]
fn stall_on_fp_load_use() {
    // FP load writing to f5; consumer reads f5 as rs1.
    let entry = IdExEntry {
        rd: 5,
        ctrl: ControlSignals {
            mem_read: true,
            fp_reg_write: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let id_ex = IdEx {
        entries: vec![entry],
    };
    let if_id = IfId {
        entries: vec![if_id_entry(5, 0)],
    };
    // fp_reg_write=true means the load targets FP file.
    // The consumer inst has rs1=5 in the encoding.
    // need_stall_load_use checks: if fp_reg_write && rd matches any of next_rs1/rs2/rs3
    assert!(
        need_stall_load_use(&id_ex, &if_id),
        "FP load to f5, use f5 → stall"
    );
}

// ══════════════════════════════════════════════════════════
// 4. Superscalar scenarios (multiple entries)
// ══════════════════════════════════════════════════════════

#[test]
fn stall_with_multiple_ex_entries() {
    // First entry: ALU (no load), second entry: load to x3
    let id_ex = IdEx {
        entries: vec![alu_entry(5), load_entry(3)],
    };
    let if_id = IfId {
        entries: vec![if_id_entry(3, 0)],
    };
    assert!(
        need_stall_load_use(&id_ex, &if_id),
        "Second EX entry is load x3, ID uses x3 → stall"
    );
}

#[test]
fn stall_detected_across_multiple_id_entries() {
    let id_ex = IdEx {
        entries: vec![load_entry(8)],
    };
    let if_id = IfId {
        entries: vec![
            if_id_entry(1, 2), // no dependency
            if_id_entry(8, 0), // depends on x8
        ],
    };
    assert!(
        need_stall_load_use(&id_ex, &if_id),
        "Second ID entry depends on load x8 → stall"
    );
}

// ══════════════════════════════════════════════════════════
// 5. All registers boundary
// ══════════════════════════════════════════════════════════

#[test]
fn stall_for_register_31() {
    let id_ex = IdEx {
        entries: vec![load_entry(31)],
    };
    let if_id = IfId {
        entries: vec![if_id_entry(31, 0)],
    };
    assert!(
        need_stall_load_use(&id_ex, &if_id),
        "Load x31, use x31 → stall"
    );
}

#[test]
fn stall_for_register_1() {
    let id_ex = IdEx {
        entries: vec![load_entry(1)],
    };
    let if_id = IfId {
        entries: vec![if_id_entry(0, 1)],
    };
    assert!(
        need_stall_load_use(&id_ex, &if_id),
        "Load x1, use x1 as rs2 → stall"
    );
}
