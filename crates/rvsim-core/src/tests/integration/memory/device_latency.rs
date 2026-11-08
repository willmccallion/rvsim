//! A device register access takes the device's access latency, as gem5's
//! `pio_latency` (100 ns by default), and a per-device override changes it.

use crate::config::BackendKind;
use crate::config::Config;
use crate::config::DeviceKind;
use crate::isa::reg::RegIdx;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{X0, X5, X6, X7};

const CLINT_MTIME_PAGE: i32 = 0x0200_B000 >> 12;
const MTIME_OFFSET: i32 = 0xFF8;
const DONE_REG: RegIdx = RegIdx::new(31);
const DONE: u64 = 7;
const CLOCK_MHZ: u64 = 2400;

/// Reads `mtime` from the CLINT, uses the value, then marks completion.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().lui(X5, CLINT_MTIME_PAGE).build(),
        i().ld(X6, X5, MTIME_OFFSET).build(),
        i().add(X7, X6, X6).build(),
        i().addi(DONE_REG, X0, DONE as i32).build(),
        i().jal(X0, 0).build(),
    ]
}

fn cycles_to_finish(configure: impl Fn(&mut Config)) -> u64 {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::InOrder;
    config.system.cpu_clock_mhz = std::num::NonZero::new(CLOCK_MHZ).unwrap();
    configure(&mut config);
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program());
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
        c.system.device_latency_ns_overrides.insert(DeviceKind::Clint, 50);
    });
    let faster_uart_only = cycles_to_finish(|c| {
        c.system.device_latency_ns_overrides.insert(DeviceKind::Uart, 50);
    });

    assert_eq!(default - faster_clint, 50 * CLOCK_MHZ / 1000);
    assert_eq!(faster_uart_only, default, "another device's override leaves the CLINT alone");
}
