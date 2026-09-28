//! Skipping idle cores changes nothing: a hart parked in WFI and woken by
//! a software interrupt gives the same run, cycle for cycle and stat for
//! stat, with its idle cycles counted instead of ticked.

use crate::common::builder::instruction::{ECALL, InstructionBuilder};
use crate::common::multihart::{DATA_BASE, MultiHart, PROGRAM_BASE};
use rvsim_core::core::pipeline::engine::BackendType;
use rvsim_core::sim::stats::StatFormat;

const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const T3: u32 = 28;
const T5: u32 = 30;
const T6: u32 = 31;
const A0: u32 = 10;
const A7: u32 = 17;
const MHARTID: u32 = 0xF14;
const MIE: u32 = 0x304;
const WFI: u32 = 0x1050_0073;
const SYS_EXIT: i32 = 93;
const WOKEN: i32 = 7;

/// Hart 1 enables its software interrupt and waits in WFI; hart 0 works,
/// raises hart 1's `msip`, waits for hart 1's flag and exits with it.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(T2, 0).build(),
        i().addi(T2, T2, (DATA_BASE - PROGRAM_BASE) as i32).build(),
        i().csrrs(T5, MHARTID, 0).build(),
        i().bne(T5, 0, 52).build(),
        i().addi(T1, 0, 300).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -4).build(),
        i().lui(T0, 0x2000).build(),
        i().addi(T3, 0, 1).build(),
        i().sw(T0, T3, 4).build(),
        i().ld(T6, T2, 0).build(),
        i().beq(T6, 0, -4).build(),
        i().sw(T0, 0, 4).build(),
        i().addi(A0, T6, 0).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
        i().addi(T3, 0, 8).build(),
        i().csrrs(0, MIE, T3).build(),
        WFI,
        i().addi(T3, 0, WOKEN).build(),
        i().sd(T2, T3, 0).build(),
        i().jal(0, 0).build(),
    ]
}

/// The exit code, the final cycle and the text of every stat.
fn run(backend: BackendType, skip_idle_cores: bool) -> (Option<u64>, u64, String) {
    let mut system = MultiHart::new(2, backend, &program());
    system.sim.skip_idle_cores = skip_idle_cores;
    let exit = system.run_until_exit(200_000);
    let mut stats = Vec::new();
    system.sim.state.stats.dump(StatFormat::Text, &mut stats).expect("dump");
    (exit, system.sim.state.cycle, String::from_utf8(stats).expect("utf-8"))
}

fn skipping_is_invisible(backend: BackendType) {
    let skipped = run(backend, true);
    let ticked = run(backend, false);

    assert_eq!(skipped.0, Some(WOKEN as u64), "hart 1 woke and wrote its flag");
    let wfi_cycles = skipped.2.lines().find_map(|l| l.strip_prefix("core1.pipeline.cycles.wfi "));
    assert!(wfi_cycles.is_some_and(|n| n.parse::<u64>().is_ok_and(|n| n > 100)), "{wfi_cycles:?}");
    assert_eq!(skipped, ticked);
}

#[test]
fn skipping_an_idle_o3_core_changes_nothing() {
    skipping_is_invisible(BackendType::OutOfOrder);
}

#[test]
fn skipping_an_idle_inorder_core_changes_nothing() {
    skipping_is_invisible(BackendType::InOrder);
}
