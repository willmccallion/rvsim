//! Skipping idle cores changes nothing: a hart parked in WFI and woken by
//! a software interrupt gives the same run, cycle for cycle and stat for
//! stat, with its idle cycles counted instead of ticked.

use crate::config::BackendKind;
use crate::sim::stats::StatFormat;
use crate::tests::support::builder::instruction::{InstructionBuilder, M5_FAIL};
use crate::tests::support::multihart::{DATA_BASE, MultiHart, PROGRAM_BASE};
use crate::tests::support::regs::{A0, T0, T1, T2, T3, T5, T6, X0, X10, X11};
const MHARTID: u32 = 0xF14;
const MIE: u32 = 0x304;
const WFI: u32 = 0x1050_0073;
const WOKEN: i32 = 7;

/// Hart 1 enables its software interrupt and waits in WFI; hart 0 works,
/// raises hart 1's `msip`, waits for hart 1's flag and exits with it.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(T2, 0).build(),
        i().addi(T2, T2, (DATA_BASE - PROGRAM_BASE) as i32).build(),
        i().csrrs(T5, MHARTID, X0).build(),
        i().bne(T5, X0, 52).build(),
        i().addi(T1, X0, 300).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, X0, -4).build(),
        i().lui(T0, 0x2000).build(),
        i().addi(T3, X0, 1).build(),
        i().sw(T0, T3, 4).build(),
        i().ld(T6, T2, 0).build(),
        i().beq(T6, X0, -4).build(),
        i().sw(T0, X0, 4).build(),
        i().addi(A0, T6, 0).build(),
        i().addi(X11, X10, 0).build(),
        M5_FAIL,
        i().addi(T3, X0, 8).build(),
        i().csrrs(X0, MIE, T3).build(),
        WFI,
        i().addi(T3, X0, WOKEN).build(),
        i().sd(T2, T3, 0).build(),
        i().jal(X0, 0).build(),
    ]
}

/// The exit code, the final cycle and the text of every stat.
fn run(backend: BackendKind, skip_idle_cores: bool) -> (Option<u64>, u64, String) {
    let mut system = MultiHart::new(2, backend, &program());
    system.sim.set_skip_idle_cores(skip_idle_cores);
    let exit = system.run_until_exit(200_000);
    let mut stats = Vec::new();
    system.sim.state.uncore.kernel.stats.dump(StatFormat::Text, &mut stats).expect("dump");
    (exit, system.sim.state.uncore.kernel.cycle, String::from_utf8(stats).expect("utf-8"))
}

fn skipping_is_invisible(backend: BackendKind) {
    let skipped = run(backend, true);
    let ticked = run(backend, false);

    assert_eq!(skipped.0, Some(WOKEN as u64), "hart 1 woke and wrote its flag");
    let wfi_cycles = skipped.2.lines().find_map(|l| l.strip_prefix("core1.pipeline.cycles.wfi "));
    assert!(wfi_cycles.is_some_and(|n| n.parse::<u64>().is_ok_and(|n| n > 100)), "{wfi_cycles:?}");
    assert_eq!(skipped, ticked);
}

#[test]
fn skipping_an_idle_o3_core_changes_nothing() {
    skipping_is_invisible(BackendKind::OutOfOrder);
}

#[test]
fn skipping_an_idle_inorder_core_changes_nothing() {
    skipping_is_invisible(BackendKind::InOrder);
}
