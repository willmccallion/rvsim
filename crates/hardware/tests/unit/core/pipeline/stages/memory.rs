//! Memory Stage Unit Tests.
//!
//! Verifies that `mem_stage` correctly performs:
//!   1. Load operations — all widths (Byte, Half, Word, Double) with sign extension
//!   2. Store operations — all widths write correct data to bus memory
//!   3. Non-memory instructions — pass through with ALU value preserved
//!   4. Trap propagation — traps pass through without memory access
//!   5. Trap-induced flush — remaining entries flushed when trap is present
//!   6. Atomic operations — LR/SC pair, AMO variants
//!   7. MEM/WB metadata — PC, inst, rd, ctrl forwarded correctly
//!   8. FP load NaN-boxing — single-precision FP loads set upper 32 bits

use crate::common::harness::TestContext;
use riscv_core::core::pipeline::latches::ExMemEntry;
use riscv_core::core::pipeline::signals::{AtomicOp, ControlSignals, MemWidth};
use riscv_core::core::pipeline::stages::mem_stage;

// ══════════════════════════════════════════════════════════
// Helpers
// ══════════════════════════════════════════════════════════

const PC: u64 = 0x8000_0000;
const MEM_BASE: u64 = 0x1000_0000;
const MEM_SIZE: usize = 0x1000;
const INST_SIZE: u64 = 4;

/// Create a TestContext with a MockMemory device attached.
fn ctx() -> TestContext {
    TestContext::new().with_memory(MEM_SIZE, MEM_BASE)
}

/// Build an ExMemEntry for a load operation.
fn load_entry(rd: usize, addr: u64, width: MemWidth, signed: bool) -> ExMemEntry {
    ExMemEntry {
        pc: PC,
        inst: 0x0000_0003, // placeholder load opcode
        inst_size: INST_SIZE,
        rd,
        alu: addr,
        store_data: 0,
        ctrl: ControlSignals {
            reg_write: true,
            mem_read: true,
            width,
            signed_load: signed,
            ..Default::default()
        },
        trap: None,
    }
}

/// Build an ExMemEntry for a store operation.
fn store_entry(addr: u64, data: u64, width: MemWidth) -> ExMemEntry {
    ExMemEntry {
        pc: PC,
        inst: 0x0000_0023, // placeholder store opcode
        inst_size: INST_SIZE,
        rd: 0,
        alu: addr,
        store_data: data,
        ctrl: ControlSignals {
            mem_write: true,
            width,
            ..Default::default()
        },
        trap: None,
    }
}

/// Build an ExMemEntry for a non-memory ALU instruction.
fn passthrough_entry(rd: usize, alu: u64) -> ExMemEntry {
    ExMemEntry {
        pc: PC,
        inst: 0x0000_0033, // placeholder R-type opcode
        inst_size: INST_SIZE,
        rd,
        alu,
        store_data: 0,
        ctrl: ControlSignals {
            reg_write: true,
            ..Default::default()
        },
        trap: None,
    }
}

/// Build an ExMemEntry for an atomic operation.
fn atomic_entry(
    rd: usize,
    addr: u64,
    store_data: u64,
    width: MemWidth,
    atomic_op: AtomicOp,
) -> ExMemEntry {
    ExMemEntry {
        pc: PC,
        inst: 0x0000_002F, // placeholder AMO opcode
        inst_size: INST_SIZE,
        rd,
        alu: addr,
        store_data,
        ctrl: ControlSignals {
            reg_write: true,
            mem_read: true,
            width,
            atomic_op,
            ..Default::default()
        },
        trap: None,
    }
}

/// Run mem_stage with a single ExMemEntry and return the first MemWbEntry.
fn mem_one(
    tc: &mut TestContext,
    entry: ExMemEntry,
) -> riscv_core::core::pipeline::latches::MemWbEntry {
    tc.cpu.ex_mem.entries = vec![entry];
    mem_stage(&mut tc.cpu);
    assert!(
        !tc.cpu.mem_wb.entries.is_empty(),
        "mem_stage should produce at least one MEM/WB entry"
    );
    tc.cpu.mem_wb.entries.remove(0)
}

// ══════════════════════════════════════════════════════════
// 1. Load operations — all widths with sign extension
// ══════════════════════════════════════════════════════════

#[test]
fn load_byte_unsigned() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u8(MEM_BASE, 0xAB);
    let wb = mem_one(&mut tc, load_entry(1, MEM_BASE, MemWidth::Byte, false));
    assert_eq!(wb.load_data, 0xAB, "LBU: zero-extended byte");
}

#[test]
fn load_byte_signed_positive() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u8(MEM_BASE, 0x7F);
    let wb = mem_one(&mut tc, load_entry(1, MEM_BASE, MemWidth::Byte, true));
    assert_eq!(wb.load_data, 0x7F, "LB: positive byte sign-extends to 0x7F");
}

#[test]
fn load_byte_signed_negative() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u8(MEM_BASE, 0x80);
    let wb = mem_one(&mut tc, load_entry(1, MEM_BASE, MemWidth::Byte, true));
    assert_eq!(wb.load_data as i64, -128, "LB: 0x80 sign-extends to -128");
}

#[test]
fn load_half_unsigned() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u16(MEM_BASE, 0xBEEF);
    let wb = mem_one(&mut tc, load_entry(1, MEM_BASE, MemWidth::Half, false));
    assert_eq!(wb.load_data, 0xBEEF, "LHU: zero-extended halfword");
}

#[test]
fn load_half_signed_negative() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u16(MEM_BASE, 0x8000);
    let wb = mem_one(&mut tc, load_entry(1, MEM_BASE, MemWidth::Half, true));
    assert_eq!(
        wb.load_data as i64, -32768,
        "LH: 0x8000 sign-extends to -32768"
    );
}

#[test]
fn load_word_unsigned() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0xDEAD_BEEF);
    let wb = mem_one(&mut tc, load_entry(1, MEM_BASE, MemWidth::Word, false));
    assert_eq!(wb.load_data, 0xDEAD_BEEF, "LWU: zero-extended word");
}

#[test]
fn load_word_signed_negative() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0x8000_0000);
    let wb = mem_one(&mut tc, load_entry(1, MEM_BASE, MemWidth::Word, true));
    assert_eq!(
        wb.load_data as i64, -2_147_483_648,
        "LW: 0x8000_0000 sign-extends to -2^31"
    );
}

#[test]
fn load_double() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u64(MEM_BASE, 0xDEAD_BEEF_CAFE_BABE);
    let wb = mem_one(&mut tc, load_entry(1, MEM_BASE, MemWidth::Double, false));
    assert_eq!(wb.load_data, 0xDEAD_BEEF_CAFE_BABE, "LD: 64-bit load");
}

// ══════════════════════════════════════════════════════════
// 2. Store operations — all widths
// ══════════════════════════════════════════════════════════

#[test]
fn store_byte() {
    let mut tc = ctx();
    mem_one(&mut tc, store_entry(MEM_BASE, 0xAB, MemWidth::Byte));
    assert_eq!(tc.cpu.bus.bus.read_u8(MEM_BASE), 0xAB, "SB: byte stored");
}

#[test]
fn store_half() {
    let mut tc = ctx();
    mem_one(&mut tc, store_entry(MEM_BASE, 0xBEEF, MemWidth::Half));
    assert_eq!(tc.cpu.bus.bus.read_u16(MEM_BASE), 0xBEEF, "SH: half stored");
}

#[test]
fn store_word() {
    let mut tc = ctx();
    mem_one(&mut tc, store_entry(MEM_BASE, 0xDEAD_BEEF, MemWidth::Word));
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE),
        0xDEAD_BEEF,
        "SW: word stored"
    );
}

#[test]
fn store_double() {
    let mut tc = ctx();
    mem_one(
        &mut tc,
        store_entry(MEM_BASE, 0xDEAD_BEEF_CAFE_BABE, MemWidth::Double),
    );
    assert_eq!(
        tc.cpu.bus.bus.read_u64(MEM_BASE),
        0xDEAD_BEEF_CAFE_BABE,
        "SD: double stored"
    );
}

#[test]
fn store_byte_truncates() {
    let mut tc = ctx();
    mem_one(&mut tc, store_entry(MEM_BASE, 0x1234_5678, MemWidth::Byte));
    assert_eq!(
        tc.cpu.bus.bus.read_u8(MEM_BASE),
        0x78,
        "SB: only lowest byte stored"
    );
}

#[test]
fn store_half_truncates() {
    let mut tc = ctx();
    mem_one(&mut tc, store_entry(MEM_BASE, 0x1234_5678, MemWidth::Half));
    assert_eq!(
        tc.cpu.bus.bus.read_u16(MEM_BASE),
        0x5678,
        "SH: only lowest halfword stored"
    );
}

// ══════════════════════════════════════════════════════════
// 3. Non-memory instructions — ALU value passes through
// ══════════════════════════════════════════════════════════

#[test]
fn alu_instruction_passes_through() {
    let mut tc = ctx();
    let wb = mem_one(&mut tc, passthrough_entry(5, 0xCAFE));
    assert_eq!(wb.alu, 0xCAFE, "ALU result forwarded to MEM/WB");
    assert_eq!(wb.load_data, 0, "No load data for ALU instruction");
    assert_eq!(wb.rd, 5);
}

#[test]
fn passthrough_preserves_metadata() {
    let mut tc = ctx();
    let entry = ExMemEntry {
        pc: 0xABCD_1234,
        inst: 0x0033_3333,
        inst_size: 4,
        rd: 7,
        alu: 42,
        store_data: 0,
        ctrl: ControlSignals {
            reg_write: true,
            ..Default::default()
        },
        trap: None,
    };
    let wb = mem_one(&mut tc, entry);
    assert_eq!(wb.pc, 0xABCD_1234, "PC preserved");
    assert_eq!(wb.inst, 0x0033_3333, "inst preserved");
    assert_eq!(wb.inst_size, 4, "inst_size preserved");
    assert_eq!(wb.rd, 7, "rd preserved");
}

// ══════════════════════════════════════════════════════════
// 4. Trap propagation
// ══════════════════════════════════════════════════════════

#[test]
fn trap_passes_through_mem_stage() {
    let mut tc = ctx();
    let trap = riscv_core::common::error::Trap::IllegalInstruction(0xDEAD);
    let entry = ExMemEntry {
        pc: PC,
        inst: 0xDEAD,
        inst_size: INST_SIZE,
        trap: Some(trap),
        ..Default::default()
    };
    let wb = mem_one(&mut tc, entry);
    assert!(wb.trap.is_some(), "Trap should propagate through MEM stage");
}

#[test]
fn trap_does_not_perform_memory_access() {
    let mut tc = ctx();
    // Write a known pattern first
    tc.cpu.bus.bus.write_u64(MEM_BASE, 0xAAAA_BBBB_CCCC_DDDD);

    let trap = riscv_core::common::error::Trap::IllegalInstruction(0xDEAD);
    let entry = ExMemEntry {
        pc: PC,
        inst: 0xDEAD,
        inst_size: INST_SIZE,
        rd: 1,
        alu: MEM_BASE,
        store_data: 0,
        ctrl: ControlSignals {
            mem_write: true,
            width: MemWidth::Double,
            ..Default::default()
        },
        trap: Some(trap),
    };
    mem_one(&mut tc, entry);

    // Memory should not have been modified
    assert_eq!(
        tc.cpu.bus.bus.read_u64(MEM_BASE),
        0xAAAA_BBBB_CCCC_DDDD,
        "Trap entry should not execute store"
    );
}

// ══════════════════════════════════════════════════════════
// 5. Trap-induced flush — remaining entries dropped
// ══════════════════════════════════════════════════════════

#[test]
fn trap_entry_flushes_remaining() {
    let mut tc = ctx();

    let trap_entry = ExMemEntry {
        pc: PC,
        trap: Some(riscv_core::common::error::Trap::IllegalInstruction(0)),
        ..Default::default()
    };
    let normal_entry = passthrough_entry(1, 99);

    tc.cpu.ex_mem.entries = vec![trap_entry, normal_entry];
    mem_stage(&mut tc.cpu);

    // Only the trap entry should produce output; the second should be flushed.
    assert_eq!(
        tc.cpu.mem_wb.entries.len(),
        1,
        "Entries after trap should be flushed"
    );
    assert!(tc.cpu.mem_wb.entries[0].trap.is_some());
}

// ══════════════════════════════════════════════════════════
// 6. Atomic operations
// ══════════════════════════════════════════════════════════

#[test]
fn lr_word_loads_and_sets_reservation() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 42);

    let entry = atomic_entry(1, MEM_BASE, 0, MemWidth::Word, AtomicOp::Lr);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data, 42, "LR.W loads the word");
    assert_eq!(
        tc.cpu.load_reservation,
        Some(MEM_BASE),
        "LR.W sets reservation"
    );
}

#[test]
fn sc_word_succeeds_with_reservation() {
    let mut tc = ctx();
    tc.cpu.load_reservation = Some(MEM_BASE);
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0);

    let entry = atomic_entry(1, MEM_BASE, 99, MemWidth::Word, AtomicOp::Sc);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data, 0, "SC.W success returns 0");
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE),
        99,
        "SC.W wrote the value"
    );
    assert_eq!(tc.cpu.load_reservation, None, "SC clears reservation");
}

#[test]
fn sc_word_fails_without_reservation() {
    let mut tc = ctx();
    tc.cpu.load_reservation = None;
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0);

    let entry = atomic_entry(1, MEM_BASE, 99, MemWidth::Word, AtomicOp::Sc);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data, 1, "SC.W failure returns 1");
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE),
        0,
        "SC.W failure does not modify memory"
    );
}

#[test]
fn sc_word_fails_with_wrong_address() {
    let mut tc = ctx();
    tc.cpu.load_reservation = Some(MEM_BASE + 8); // different address
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0);

    let entry = atomic_entry(1, MEM_BASE, 99, MemWidth::Word, AtomicOp::Sc);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data, 1, "SC.W fails on address mismatch");
}

#[test]
fn lr_sc_double_pair() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u64(MEM_BASE, 100);

    // LR.D
    let lr = atomic_entry(1, MEM_BASE, 0, MemWidth::Double, AtomicOp::Lr);
    let wb_lr = mem_one(&mut tc, lr);
    assert_eq!(wb_lr.load_data, 100, "LR.D loads the double");
    assert_eq!(tc.cpu.load_reservation, Some(MEM_BASE));

    // SC.D
    let sc = atomic_entry(1, MEM_BASE, 200, MemWidth::Double, AtomicOp::Sc);
    let wb_sc = mem_one(&mut tc, sc);
    assert_eq!(wb_sc.load_data, 0, "SC.D success returns 0");
    assert_eq!(tc.cpu.bus.bus.read_u64(MEM_BASE), 200, "SC.D wrote 200");
}

#[test]
fn amoadd_word() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 10);

    let entry = atomic_entry(1, MEM_BASE, 5, MemWidth::Word, AtomicOp::Add);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data, 10, "AMOADD.W returns old value");
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE),
        15,
        "AMOADD.W stores old + rs2"
    );
}

#[test]
fn amoswap_word() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 42);

    let entry = atomic_entry(1, MEM_BASE, 99, MemWidth::Word, AtomicOp::Swap);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data, 42, "AMOSWAP.W returns old value");
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE),
        99,
        "AMOSWAP.W stores new value"
    );
}

#[test]
fn amoxor_word() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0xFF00);

    let entry = atomic_entry(1, MEM_BASE, 0x0FF0, MemWidth::Word, AtomicOp::Xor);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data as u32, 0xFF00, "AMOXOR.W returns old value");
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE),
        0xF0F0,
        "AMOXOR.W stores XOR result"
    );
}

#[test]
fn amoand_word() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0xFF00);

    let entry = atomic_entry(1, MEM_BASE, 0x0FF0, MemWidth::Word, AtomicOp::And);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data as u32, 0xFF00, "AMOAND.W returns old value");
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE),
        0x0F00,
        "AMOAND.W stores AND result"
    );
}

#[test]
fn amoor_word() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0xFF00);

    let entry = atomic_entry(1, MEM_BASE, 0x00FF, MemWidth::Word, AtomicOp::Or);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data as u32, 0xFF00, "AMOOR.W returns old value");
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE),
        0xFFFF,
        "AMOOR.W stores OR result"
    );
}

#[test]
fn amomin_word() {
    let mut tc = ctx();
    // Store -5 as i32 in memory
    tc.cpu.bus.bus.write_u32(MEM_BASE, (-5_i32) as u32);

    // AMO min with 3 (signed)
    let entry = atomic_entry(1, MEM_BASE, 3, MemWidth::Word, AtomicOp::Min);
    let wb = mem_one(&mut tc, entry);

    // Old value was -5 (sign-extended)
    assert_eq!(wb.load_data as i32, -5, "AMOMIN.W returns old value");
    // min(-5, 3) = -5
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE) as i32,
        -5,
        "AMOMIN.W stores min(-5, 3) = -5"
    );
}

#[test]
fn amomax_word() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, (-5_i32) as u32);

    let entry = atomic_entry(1, MEM_BASE, 3, MemWidth::Word, AtomicOp::Max);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data as i32, -5, "AMOMAX.W returns old value");
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE) as i32,
        3,
        "AMOMAX.W stores max(-5, 3) = 3"
    );
}

#[test]
fn amominu_word() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 10);

    let entry = atomic_entry(1, MEM_BASE, 3, MemWidth::Word, AtomicOp::Minu);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data as u32, 10, "AMOMINU.W returns old value");
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE),
        3,
        "AMOMINU.W stores minu(10, 3) = 3"
    );
}

#[test]
fn amomaxu_word() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 10);

    let entry = atomic_entry(1, MEM_BASE, 300, MemWidth::Word, AtomicOp::Maxu);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data as u32, 10, "AMOMAXU.W returns old value");
    assert_eq!(
        tc.cpu.bus.bus.read_u32(MEM_BASE),
        300,
        "AMOMAXU.W stores maxu(10, 300) = 300"
    );
}

#[test]
fn amoadd_double() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u64(MEM_BASE, 1000);

    let entry = atomic_entry(1, MEM_BASE, 234, MemWidth::Double, AtomicOp::Add);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(wb.load_data, 1000, "AMOADD.D returns old value");
    assert_eq!(
        tc.cpu.bus.bus.read_u64(MEM_BASE),
        1234,
        "AMOADD.D stores 1000 + 234 = 1234"
    );
}

#[test]
fn amo_clears_reservation_on_matching_address() {
    let mut tc = ctx();
    tc.cpu.load_reservation = Some(MEM_BASE);
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0);

    let entry = atomic_entry(1, MEM_BASE, 5, MemWidth::Word, AtomicOp::Add);
    mem_one(&mut tc, entry);

    assert_eq!(
        tc.cpu.load_reservation, None,
        "AMO clears reservation on matching address"
    );
}

// ══════════════════════════════════════════════════════════
// 7. Store clears load reservation on matching address
// ══════════════════════════════════════════════════════════

#[test]
fn store_clears_load_reservation() {
    let mut tc = ctx();
    tc.cpu.load_reservation = Some(MEM_BASE);

    mem_one(&mut tc, store_entry(MEM_BASE, 42, MemWidth::Word));
    assert_eq!(
        tc.cpu.load_reservation, None,
        "Store to reserved address clears reservation"
    );
}

#[test]
fn store_does_not_clear_unrelated_reservation() {
    let mut tc = ctx();
    tc.cpu.load_reservation = Some(MEM_BASE + 0x100);

    mem_one(&mut tc, store_entry(MEM_BASE, 42, MemWidth::Word));
    assert_eq!(
        tc.cpu.load_reservation,
        Some(MEM_BASE + 0x100),
        "Store to different address preserves reservation"
    );
}

// ══════════════════════════════════════════════════════════
// 8. Multiple entries execute in order
// ══════════════════════════════════════════════════════════

#[test]
fn multiple_entries_all_produce_results() {
    let mut tc = ctx();

    tc.cpu.bus.bus.write_u32(MEM_BASE, 0xAABB);

    let e1 = load_entry(1, MEM_BASE, MemWidth::Word, false);
    let e2 = passthrough_entry(2, 999);

    tc.cpu.ex_mem.entries = vec![e1, e2];
    mem_stage(&mut tc.cpu);

    assert_eq!(tc.cpu.mem_wb.entries.len(), 2);
    assert_eq!(tc.cpu.mem_wb.entries[0].load_data, 0xAABB);
    assert_eq!(tc.cpu.mem_wb.entries[1].alu, 999);
}

#[test]
fn store_then_load_sees_stored_value() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0); // clear

    let st = store_entry(MEM_BASE, 0xDEAD, MemWidth::Word);
    let ld = load_entry(1, MEM_BASE, MemWidth::Word, false);

    tc.cpu.ex_mem.entries = vec![st, ld];
    mem_stage(&mut tc.cpu);

    assert_eq!(tc.cpu.mem_wb.entries.len(), 2);
    assert_eq!(
        tc.cpu.mem_wb.entries[1].load_data, 0xDEAD,
        "Load after store in same cycle sees stored value"
    );
}

// ══════════════════════════════════════════════════════════
// 9. Empty EX/MEM produces empty MEM/WB
// ══════════════════════════════════════════════════════════

#[test]
fn empty_ex_mem_produces_empty_mem_wb() {
    let mut tc = ctx();
    tc.cpu.ex_mem.entries.clear();
    mem_stage(&mut tc.cpu);
    assert!(
        tc.cpu.mem_wb.entries.is_empty(),
        "Empty EX/MEM → empty MEM/WB"
    );
}

// ══════════════════════════════════════════════════════════
// 10. Load at non-zero offset
// ══════════════════════════════════════════════════════════

#[test]
fn load_at_offset() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE + 0x10, 0xCAFE);
    let wb = mem_one(
        &mut tc,
        load_entry(1, MEM_BASE + 0x10, MemWidth::Word, false),
    );
    assert_eq!(wb.load_data, 0xCAFE, "Load from base+0x10");
}

// ══════════════════════════════════════════════════════════
// 11. FP load NaN-boxing
// ══════════════════════════════════════════════════════════

#[test]
fn fp_single_load_nan_boxes() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0x3F80_0000); // 1.0f

    let entry = ExMemEntry {
        pc: PC,
        inst: 0x0000_0007, // placeholder FLW
        inst_size: INST_SIZE,
        rd: 1,
        alu: MEM_BASE,
        store_data: 0,
        ctrl: ControlSignals {
            fp_reg_write: true,
            mem_read: true,
            width: MemWidth::Word,
            ..Default::default()
        },
        trap: None,
    };
    let wb = mem_one(&mut tc, entry);

    assert_eq!(
        wb.load_data, 0xFFFF_FFFF_3F80_0000,
        "FLW NaN-boxes: upper 32 bits set to 1s"
    );
}

// ══════════════════════════════════════════════════════════
// 12. LR sign-extends word
// ══════════════════════════════════════════════════════════

#[test]
fn lr_word_sign_extends_negative() {
    let mut tc = ctx();
    tc.cpu.bus.bus.write_u32(MEM_BASE, 0x8000_0000);

    let entry = atomic_entry(1, MEM_BASE, 0, MemWidth::Word, AtomicOp::Lr);
    let wb = mem_one(&mut tc, entry);

    assert_eq!(
        wb.load_data as i64, -2_147_483_648,
        "LR.W sign-extends negative word"
    );
}
