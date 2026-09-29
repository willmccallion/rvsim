//! Loads that partially overlap an older uncommitted store.
//!
//! Such a load cannot be forwarded and must wait for that store to drain.
//! While it waits, older memory operations must keep issuing; otherwise the
//! store it waits on can never reach the ROB head and the machine deadlocks.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x4000;
const DATA_BASE: u64 = RAM_BASE + 0x1000;
const X6: u32 = 6;
const X8: u32 = 8;
const X28: u32 = 28;
const X31: u32 = 31;

/// (store width in bytes, store offset, load width in bytes, load offset).
/// Every pair overlaps without the store fully covering the load, so the
/// load cannot be forwarded from the store buffer.
const PARTIAL_OVERLAPS: [(u8, i32, u8, i32); 6] = [
    (4, 32, 8, 28),
    (4, 36, 8, 32),
    (8, 40, 4, 44),
    (8, 48, 8, 52),
    (4, 60, 8, 56),
    (8, 64, 4, 68),
];

fn store(width: u8, offset: i32) -> u32 {
    match width {
        4 => InstructionBuilder::new().sw(X8, X6, offset).build(),
        8 => InstructionBuilder::new().sd(X8, X6, offset).build(),
        _ => unreachable!("unsupported store width"),
    }
}

fn load(width: u8, offset: i32) -> u32 {
    match width {
        4 => InstructionBuilder::new().lw(X28, X8, offset).build(),
        8 => InstructionBuilder::new().ld(X28, X8, offset).build(),
        _ => unreachable!("unsupported load width"),
    }
}

/// Mirrors the riscv-tests `ma_data` MISMATCHED_STORE_TEST shape: back to
/// back store/load pairs with partial overlap, then a marker write to x31.
fn partial_overlap_program() -> Vec<u32> {
    let mut program = Vec::new();
    for _ in 0..4 {
        for (i, &(sw, so, lw, lo)) in PARTIAL_OVERLAPS.iter().enumerate() {
            program.push(InstructionBuilder::new().addi(X6, 0, (i as i32 + 1) * 0x111).build());
            program.push(store(sw, so));
            program.push(load(lw, lo));
        }
    }
    program.push(InstructionBuilder::new().addi(X31, 0, 1).build());
    program.push(InstructionBuilder::new().jal(0, 0).build());
    program
}

fn run_program(config: &Config) -> TestContext {
    let mut tc = TestContext::new_with_config(config)
        .with_memory(RAM_SIZE, RAM_BASE)
        .load_program(RAM_BASE, &partial_overlap_program());
    tc.set_reg(X8 as usize, DATA_BASE);
    tc.sim.sync_arch_regs();
    tc.run(20_000);
    tc
}

#[test]
fn out_of_order_backend_completes_partially_overlapping_store_load_pairs() {
    let mut config = Config::default();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.width = 4;

    let tc = run_program(&config);

    assert_eq!(tc.get_reg(X31 as usize), 1, "program never reached its marker: deadlocked");
}

#[test]
fn in_order_backend_completes_partially_overlapping_store_load_pairs() {
    let mut config = Config::default();
    config.pipeline.backend = BackendKind::InOrder;
    config.pipeline.width = 4;

    let tc = run_program(&config);

    assert_eq!(tc.get_reg(X31 as usize), 1, "program never reached its marker: deadlocked");
}
