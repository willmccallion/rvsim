//! Skipping the cycles in which nothing but time passes changes nothing.
//!
//! Harts asleep on a timer, DDR5 refreshing, powering down and scrubbing,
//! and an idle crossbar give the same run, cycle for cycle and stat for
//! stat, whether the quiet stretches are ticked or skipped.

use crate::common::builder::instruction::{ECALL, InstructionBuilder};
use crate::common::multihart::MultiHart;
use rvsim_core::config::{Config, Console, InterconnectConfig, MemoryControllerKind};
use rvsim_core::sim::stats::StatFormat;
use rvsim_core::system::simulator::{StopAt, StopReason};

const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const T3: u32 = 28;
const T5: u32 = 30;
const T6: u32 = 31;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A7: u32 = 17;
const MHARTID: u32 = 0xF14;
const MIE: u32 = 0x304;
const MTIE: i32 = 0x80;
const MSIE: i32 = 0x8;
const WFI: u32 = 0x1050_0073;
const SYS_EXIT: i32 = 93;
const EXIT_CODE: i32 = 5;
/// Timer ticks each sleep lasts; with the CLINT divider, many thousands
/// of cycles, long enough for DDR5 refreshes and power-down.
const SLEEP_TICKS: i32 = 1500;
const CLINT_DIVIDER: u64 = 10;

/// Hart 0 sleeps on its timer, stores 64 lines, sleeps again and exits;
/// hart 1 waits for a software interrupt that never comes. Each sleep
/// re-checks `mtime`: the store moving `mtimecmp` may still be buffered
/// when the WFI issues, leaving the expired comparator's interrupt pending.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let set_timer = |i: fn() -> InstructionBuilder| {
        [
            i().ld(T6, T0, -8).build(),
            i().addi(T6, T6, SLEEP_TICKS).build(),
            i().sd(T1, T6, 0).build(),
            WFI,
            i().ld(A2, T0, -8).build(),
            i().bltu(A2, T6, -8).build(),
        ]
    };
    let mut words = vec![
        i().csrrs(T5, MHARTID, 0).build(),
        i().bne(T5, 0, 104).build(),
        i().lui(T0, 0x200C).build(),
        i().lui(T1, 0x2004).build(),
        i().addi(T3, 0, MTIE).build(),
        i().csrrs(0, MIE, T3).build(),
    ];
    words.extend(set_timer(InstructionBuilder::new));
    words.extend([
        i().auipc(T2, 0x10).build(),
        i().addi(A1, 0, 64).build(),
        i().sd(T2, T3, 0).build(),
        i().addi(T2, T2, 64).build(),
        i().addi(A1, A1, -1).build(),
        i().bne(A1, 0, -12).build(),
    ]);
    words.extend(set_timer(InstructionBuilder::new));
    words.extend([
        i().addi(A0, 0, EXIT_CODE).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
        i().addi(T3, 0, MSIE).build(),
        i().csrrs(0, MIE, T3).build(),
        WFI,
        i().jal(0, -4).build(),
    ]);
    assert_eq!(words.len(), 31, "the park loop sits where hart 1 branches to");
    words
}

fn config() -> Config {
    let mut config = Config::default();
    config.system.hart_count = 2;
    config.system.console = Console::Quiet;
    config.system.clint_divider = CLINT_DIVIDER;
    config.memory.controller = MemoryControllerKind::Ddr5;
    config.memory.ddr5 = serde_json::from_str(
        r#"{"power_down_idle_ns": 50, "ecc": "SecDed", "patrol_scrub_ns": 5000}"#,
    )
    .expect("DDR5 parameters");
    config.coherence.interconnect =
        InterconnectConfig::Crossbar { hop_latency: 2, bytes_per_cycle: 8 };
    config
}

/// The exit code, the final cycle, `mcycle` on each hart and every stat.
fn run(skip_idle_cores: bool) -> (StopReason, u64, Vec<u64>, String) {
    let mut system = MultiHart::with_config(&config(), &program());
    system.sim.skip_idle_cores = skip_idle_cores;
    let reason = system
        .sim
        .run_to(&StopAt { cycles: Some(2_000_000), ..StopAt::default() })
        .expect("the run ticks");
    let mcycle = system.sim.state.harts.iter().map(|hart| hart.csrs.mcycle).collect();
    let mut stats = Vec::new();
    system.sim.state.stats.dump(StatFormat::Text, &mut stats).expect("dump");
    (reason, system.sim.state.cycle, mcycle, String::from_utf8(stats).expect("utf-8"))
}

fn stat(stats: &str, path: &str) -> u64 {
    stats
        .lines()
        .find_map(|line| line.strip_prefix(path)?.trim().parse().ok())
        .unwrap_or_else(|| panic!("no stat {path}"))
}

#[test]
fn skipping_quiet_cycles_gives_the_run_ticking_would() {
    let skipped = run(true);
    let ticked = run(false);

    assert_eq!(skipped.0, StopReason::Exited(EXIT_CODE as u64));
    let sleep_cycles = 2 * SLEEP_TICKS as u64 * CLINT_DIVIDER;
    assert!(skipped.1 > sleep_cycles, "both sleeps elapsed: {}", skipped.1);
    assert!(stat(&skipped.3, "memctrl0.ch0.sc0.refreshes") > 0);
    assert!(stat(&skipped.3, "memctrl0.ch0.sc0.power_down_entries") > 0);
    assert_eq!(skipped, ticked);
}
