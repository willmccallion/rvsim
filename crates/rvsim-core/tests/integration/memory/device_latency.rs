//! A device register access takes the device's access latency, as gem5's
//! `pio_latency` (100 ns by default), and a per-device override changes it.

use crate::support::builder::instruction::InstructionBuilder;
use crate::support::harness::TestContext;
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;

const PROGRAM_BASE: u64 = 0x8000_0000;
const CLINT_MTIME_PAGE: i32 = 0x0200_B000 >> 12;
const MTIME_OFFSET: i32 = 0xFF8;
const DONE_REG: usize = 31;
const DONE: u64 = 7;
const CLOCK_MHZ: u64 = 2400;

/// Reads `mtime` from the CLINT, uses the value, then marks completion.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().lui(5, CLINT_MTIME_PAGE).build(),
        i().ld(6, 5, MTIME_OFFSET).build(),
        i().add(7, 6, 6).build(),
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ]
}

fn cycles_to_finish(configure: impl Fn(&mut Config)) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = BackendKind::InOrder;
    config.system.cpu_clock_mhz = CLOCK_MHZ;
    configure(&mut config);
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    ctx.run_until(5_000, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished")
}

#[test]
fn a_device_read_takes_the_device_access_latency() {
    let instant = cycles_to_finish(|c| c.system.device_latency_ns = 0);
    let default = cycles_to_finish(|_| {});

    let expected = 100 * CLOCK_MHZ / 1000;
    assert_eq!(default - instant, expected, "100 ns at {CLOCK_MHZ} MHz");
}

#[test]
fn a_per_device_latency_overrides_the_default() {
    let default = cycles_to_finish(|_| {});
    let faster_clint = cycles_to_finish(|c| {
        c.system.device_latency_ns_overrides.insert("CLINT".to_string(), 50);
    });
    let faster_uart_only = cycles_to_finish(|c| {
        c.system.device_latency_ns_overrides.insert("UART0".to_string(), 50);
    });

    assert_eq!(default - faster_clint, 50 * CLOCK_MHZ / 1000);
    assert_eq!(faster_uart_only, default, "another device's override leaves the CLINT alone");
}
