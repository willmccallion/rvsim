//! Data Forwarding Tests — RAW Hazard Resolution.
//!
//! Verifies that `forward_rs` correctly bypasses register values from
//! later pipeline stages (EX/MEM, MEM/WB fresh, MEM/WB old, intra-bundle)
//! to resolve Read-After-Write (RAW) data hazards.

use riscv_core::core::pipeline::hazards::forward_rs;
use riscv_core::core::pipeline::latches::{ExMem, ExMemEntry, IdExEntry, MemWb, MemWbEntry};
use riscv_core::core::pipeline::signals::ControlSignals;

/// Helper: create an IdExEntry that reads from given integer registers.
fn consumer(rs1: usize, rs2: usize) -> IdExEntry {
    IdExEntry {
        rs1,
        rs2,
        rs3: 0,
        rv1: 0xDEAD_0001,
        rv2: 0xDEAD_0002,
        rv3: 0,
        ctrl: ControlSignals::default(),
        ..Default::default()
    }
}

/// Helper: create an ExMemEntry that writes an ALU result to rd.
fn ex_producer(rd: usize, alu: u64) -> ExMemEntry {
    ExMemEntry {
        rd,
        alu,
        ctrl: ControlSignals {
            reg_write: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Helper: create a MemWbEntry that writes an ALU result to rd.
fn wb_producer(rd: usize, alu: u64) -> MemWbEntry {
    MemWbEntry {
        rd,
        alu,
        ctrl: ControlSignals {
            reg_write: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Helper: create a MemWbEntry from a load (mem_read).
fn wb_load_producer(rd: usize, load_data: u64) -> MemWbEntry {
    MemWbEntry {
        rd,
        load_data,
        ctrl: ControlSignals {
            reg_write: true,
            mem_read: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Helper: create a MemWbEntry from a jump (link register).
fn wb_jump_producer(rd: usize, pc: u64, inst_size: u64) -> MemWbEntry {
    MemWbEntry {
        rd,
        pc,
        inst_size,
        ctrl: ControlSignals {
            reg_write: true,
            jump: true,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Empty pipeline latches.
fn empty_ex_mem() -> ExMem {
    ExMem { entries: vec![] }
}

fn empty_mem_wb() -> MemWb {
    MemWb { entries: vec![] }
}

// ══════════════════════════════════════════════════════════
// 1. No forwarding needed — values come from register file
// ══════════════════════════════════════════════════════════

#[test]
fn no_forwarding_returns_regfile_values() {
    let id = consumer(1, 2);
    let (a, b, _) = forward_rs(
        &id,
        &empty_ex_mem(),
        &empty_mem_wb(),
        &empty_mem_wb(),
        &[],
        false,
    );
    assert_eq!(a, 0xDEAD_0001, "rs1 should come from regfile");
    assert_eq!(b, 0xDEAD_0002, "rs2 should come from regfile");
}

// ══════════════════════════════════════════════════════════
// 2. EX/MEM forwarding (1-cycle-old ALU result)
// ══════════════════════════════════════════════════════════

#[test]
fn forward_from_ex_mem_to_rs1() {
    let id = consumer(5, 6);
    let ex_mem = ExMem {
        entries: vec![ex_producer(5, 0x1111)],
    };
    let (a, b, _) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &empty_mem_wb(), &[], false);
    assert_eq!(a, 0x1111, "rs1=x5 should forward from EX/MEM");
    assert_eq!(b, 0xDEAD_0002, "rs2=x6 should remain regfile");
}

#[test]
fn forward_from_ex_mem_to_rs2() {
    let id = consumer(5, 6);
    let ex_mem = ExMem {
        entries: vec![ex_producer(6, 0x2222)],
    };
    let (a, b, _) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &empty_mem_wb(), &[], false);
    assert_eq!(a, 0xDEAD_0001);
    assert_eq!(b, 0x2222, "rs2=x6 should forward from EX/MEM");
}

#[test]
fn forward_from_ex_mem_to_both() {
    let id = consumer(3, 3);
    let ex_mem = ExMem {
        entries: vec![ex_producer(3, 0x3333)],
    };
    let (a, b, _) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &empty_mem_wb(), &[], false);
    assert_eq!(a, 0x3333);
    assert_eq!(b, 0x3333);
}

#[test]
fn ex_mem_load_does_not_forward() {
    // A load in EX/MEM hasn't completed yet — data isn't available.
    // forward_rs skips entries with mem_read=true in EX/MEM.
    let id = consumer(5, 6);
    let mut entry = ex_producer(5, 0xBAD);
    entry.ctrl.mem_read = true;
    let ex_mem = ExMem {
        entries: vec![entry],
    };
    let (a, _, _) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &empty_mem_wb(), &[], false);
    assert_ne!(
        a, 0xBAD,
        "Load result from EX/MEM must not be forwarded (not yet available)"
    );
}

// ══════════════════════════════════════════════════════════
// 3. MEM/WB fresh forwarding (current-cycle memory result)
// ══════════════════════════════════════════════════════════

#[test]
fn forward_from_mem_wb_fresh_alu() {
    let id = consumer(7, 8);
    let mem_wb_fresh = MemWb {
        entries: vec![wb_producer(7, 0xAAAA)],
    };
    let (a, b, _) = forward_rs(
        &id,
        &empty_ex_mem(),
        &empty_mem_wb(),
        &mem_wb_fresh,
        &[],
        false,
    );
    assert_eq!(a, 0xAAAA, "rs1=x7 should forward from MEM/WB fresh");
    assert_eq!(b, 0xDEAD_0002);
}

#[test]
fn forward_from_mem_wb_fresh_load_data() {
    let id = consumer(7, 8);
    let mem_wb_fresh = MemWb {
        entries: vec![wb_load_producer(7, 0xBBBB)],
    };
    let (a, _, _) = forward_rs(
        &id,
        &empty_ex_mem(),
        &empty_mem_wb(),
        &mem_wb_fresh,
        &[],
        false,
    );
    assert_eq!(a, 0xBBBB, "Load data should be forwarded from MEM/WB fresh");
}

#[test]
fn forward_from_mem_wb_fresh_jump() {
    let id = consumer(1, 2);
    let mem_wb_fresh = MemWb {
        entries: vec![wb_jump_producer(1, 0x1000, 4)],
    };
    let (a, _, _) = forward_rs(
        &id,
        &empty_ex_mem(),
        &empty_mem_wb(),
        &mem_wb_fresh,
        &[],
        false,
    );
    assert_eq!(
        a, 0x1004,
        "Jump link (pc+4) should be forwarded from MEM/WB fresh"
    );
}

// ══════════════════════════════════════════════════════════
// 4. MEM/WB old forwarding (2-cycle-old result)
// ══════════════════════════════════════════════════════════

#[test]
fn forward_from_mem_wb_old() {
    let id = consumer(10, 11);
    let mem_wb_old = MemWb {
        entries: vec![wb_producer(10, 0xCCCC)],
    };
    let (a, _, _) = forward_rs(
        &id,
        &empty_ex_mem(),
        &mem_wb_old,
        &empty_mem_wb(),
        &[],
        false,
    );
    assert_eq!(a, 0xCCCC, "rs1=x10 should forward from MEM/WB old");
}

// ══════════════════════════════════════════════════════════
// 5. Priority: EX/MEM > MEM/WB fresh > MEM/WB old
// ══════════════════════════════════════════════════════════

#[test]
fn ex_mem_takes_priority_over_mem_wb_fresh() {
    let id = consumer(5, 6);
    let ex_mem = ExMem {
        entries: vec![ex_producer(5, 0x1111)],
    };
    let mem_wb_fresh = MemWb {
        entries: vec![wb_producer(5, 0x2222)],
    };
    let (a, _, _) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &mem_wb_fresh, &[], false);
    assert_eq!(a, 0x1111, "EX/MEM (most recent) should take priority");
}

#[test]
fn mem_wb_fresh_takes_priority_over_mem_wb_old() {
    let id = consumer(5, 6);
    let mem_wb_old = MemWb {
        entries: vec![wb_producer(5, 0x3333)],
    };
    let mem_wb_fresh = MemWb {
        entries: vec![wb_producer(5, 0x4444)],
    };
    let (a, _, _) = forward_rs(&id, &empty_ex_mem(), &mem_wb_old, &mem_wb_fresh, &[], false);
    assert_eq!(a, 0x4444, "MEM/WB fresh should take priority over old");
}

#[test]
fn full_priority_chain() {
    // All stages produce values for the same register; EX/MEM wins.
    let id = consumer(5, 5);
    let ex_mem = ExMem {
        entries: vec![ex_producer(5, 0xEE)],
    };
    let mem_wb_fresh = MemWb {
        entries: vec![wb_producer(5, 0xFF)],
    };
    let mem_wb_old = MemWb {
        entries: vec![wb_producer(5, 0xDD)],
    };
    let (a, b, _) = forward_rs(&id, &ex_mem, &mem_wb_old, &mem_wb_fresh, &[], false);
    assert_eq!(a, 0xEE);
    assert_eq!(b, 0xEE);
}

// ══════════════════════════════════════════════════════════
// 6. Intra-bundle forwarding (superscalar)
// ══════════════════════════════════════════════════════════

#[test]
fn intra_bundle_forward_rs1() {
    let id = consumer(5, 6);
    let bundle = vec![ex_producer(5, 0x5555)];
    let (a, b, _) = forward_rs(
        &id,
        &empty_ex_mem(),
        &empty_mem_wb(),
        &empty_mem_wb(),
        &bundle,
        false,
    );
    assert_eq!(a, 0x5555, "Intra-bundle result should forward to rs1");
    assert_eq!(b, 0xDEAD_0002);
}

#[test]
fn intra_bundle_takes_highest_priority() {
    let id = consumer(5, 6);
    let ex_mem = ExMem {
        entries: vec![ex_producer(5, 0x1111)],
    };
    let bundle = vec![ex_producer(5, 0x9999)];
    let (a, _, _) = forward_rs(
        &id,
        &ex_mem,
        &empty_mem_wb(),
        &empty_mem_wb(),
        &bundle,
        false,
    );
    assert_eq!(a, 0x9999, "Intra-bundle should override EX/MEM");
}

#[test]
fn intra_bundle_earlier_instruction_takes_effect() {
    // In a superscalar bundle, forward_rs iterates in reverse so the
    // earliest instruction (program order) in the bundle is the last
    // to overwrite, giving it effective priority for same-register writes.
    let id = consumer(5, 6);
    let bundle = vec![
        ex_producer(5, 0xAAAA), // first in program order
        ex_producer(5, 0xBBBB), // second in program order
    ];
    let (a, _, _) = forward_rs(
        &id,
        &empty_ex_mem(),
        &empty_mem_wb(),
        &empty_mem_wb(),
        &bundle,
        false,
    );
    assert_eq!(
        a, 0xAAAA,
        "Earliest instruction in bundle takes effect after reverse iteration"
    );
}

// ══════════════════════════════════════════════════════════
// 7. x0 is never forwarded (hardwired zero)
// ══════════════════════════════════════════════════════════

#[test]
fn x0_never_forwarded() {
    let mut id = consumer(0, 0);
    id.rv1 = 0;
    id.rv2 = 0;
    let ex_mem = ExMem {
        entries: vec![ex_producer(0, 0xFFFF)],
    };
    let (a, b, _) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &empty_mem_wb(), &[], false);
    assert_eq!(a, 0, "x0 must not be forwarded — always 0");
    assert_eq!(b, 0, "x0 must not be forwarded — always 0");
}

// ══════════════════════════════════════════════════════════
// 8. Floating-point forwarding
// ══════════════════════════════════════════════════════════

#[test]
fn fp_forward_from_ex_mem() {
    let mut id = consumer(1, 2);
    id.ctrl.rs1_fp = true;
    id.rv1 = 0xDEAD_F0;
    let mut producer = ex_producer(1, 0xF100);
    producer.ctrl.fp_reg_write = true;
    producer.ctrl.reg_write = false;
    let ex_mem = ExMem {
        entries: vec![producer],
    };
    let (a, _, _) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &empty_mem_wb(), &[], false);
    assert_eq!(a, 0xF100, "FP register should forward from EX/MEM");
}

#[test]
fn fp_and_int_registers_are_separate() {
    // x5 (int) and f5 (fp) are different register files.
    // A producer writing to int x5 should NOT forward to a consumer reading fp f5.
    let mut id = consumer(5, 6);
    id.ctrl.rs1_fp = true;
    id.rv1 = 0xF0_0000;

    let producer = ex_producer(5, 0xA0_0000);
    let ex_mem = ExMem {
        entries: vec![producer],
    };
    let (a, _, _) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &empty_mem_wb(), &[], false);
    assert_ne!(a, 0xA0_0000, "Int write to x5 must not forward to fp f5");
}

// ══════════════════════════════════════════════════════════
// 9. Trap entries are skipped
// ══════════════════════════════════════════════════════════

#[test]
fn trap_entries_are_not_forwarded() {
    let id = consumer(5, 6);
    let mut trapped = ex_producer(5, 0xBAD);
    trapped.trap = Some(riscv_core::common::error::Trap::IllegalInstruction(0));
    let ex_mem = ExMem {
        entries: vec![trapped],
    };
    let (a, _, _) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &empty_mem_wb(), &[], false);
    assert_eq!(
        a, 0xDEAD_0001,
        "Trapped instruction must not forward its result"
    );
}

// ══════════════════════════════════════════════════════════
// 10. Mixed rs1/rs2 from different sources
// ══════════════════════════════════════════════════════════

#[test]
fn rs1_from_ex_mem_rs2_from_mem_wb() {
    let id = consumer(5, 7);
    let ex_mem = ExMem {
        entries: vec![ex_producer(5, 0xAA)],
    };
    let mem_wb_fresh = MemWb {
        entries: vec![wb_producer(7, 0xBB)],
    };
    let (a, b, _) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &mem_wb_fresh, &[], false);
    assert_eq!(a, 0xAA, "rs1 from EX/MEM");
    assert_eq!(b, 0xBB, "rs2 from MEM/WB fresh");
}

// ══════════════════════════════════════════════════════════
// 11. rs3 forwarding (FMA instructions)
// ══════════════════════════════════════════════════════════

#[test]
fn rs3_forward_from_ex_mem() {
    let id = IdExEntry {
        rs1: 1,
        rs2: 2,
        rs3: 3,
        rv1: 0,
        rv2: 0,
        rv3: 0xDEAD_0003,
        ctrl: ControlSignals {
            rs3_fp: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut producer = ex_producer(3, 0xF0AC);
    producer.ctrl.fp_reg_write = true;
    producer.ctrl.reg_write = false;
    let ex_mem = ExMem {
        entries: vec![producer],
    };
    let (_, _, c) = forward_rs(&id, &ex_mem, &empty_mem_wb(), &empty_mem_wb(), &[], false);
    assert_eq!(
        c, 0xF0AC,
        "rs3 should forward from EX/MEM for FMA instructions"
    );
}
