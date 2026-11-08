//! Each device's registers survive a checkpoint: what it reports after a
//! restore is what it reported before.

use crate::common::{HartId, PhysAddr};
use crate::config::Config;
use crate::config::Console;
use crate::soc::devices::Device;
use crate::soc::devices::clint::Clint;
use crate::soc::devices::plic::Plic;
use crate::soc::devices::uart::Uart;
use crate::soc::devices::virtio_disk::VirtioBlock;
use crate::soc::terminal::Terminal;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::harness::build_simulator;

const CLINT_BASE: u64 = 0x0200_0000;
const PLIC_BASE: u64 = 0x0c00_0000;
const UART_BASE: u64 = 0x1000_0000;
const VIRTIO_BASE: u64 = 0x1000_1000;

fn write(device: &mut impl Device, addr: u64, value: u64, width: ProbeWidth) {
    crate::tests::support::probe::write(device, PhysAddr::new(addr), value, width);
}

fn read(device: &mut impl Device, addr: u64, width: ProbeWidth) -> u64 {
    crate::tests::support::probe::read(device, PhysAddr::new(addr), width)
}

/// Moves `device`'s checkpoint into a fresh `blank` and returns it.
fn restored<D: Device>(device: &D, mut blank: D) -> D {
    let state = device.checkpoint().expect("the device has state to checkpoint");
    blank.restore(&state).expect("the device takes its own state");
    assert_eq!(blank.checkpoint(), Some(state), "the restored device checkpoints identically");
    blank
}

#[test]
fn a_clint_keeps_its_clock_timers_and_software_interrupts() {
    let mut clint = Clint::new(PhysAddr::new(CLINT_BASE), std::num::NonZero::new(1).unwrap(), 2);
    write(&mut clint, CLINT_BASE + 0x4008, 40, ProbeWidth::B8);
    write(&mut clint, CLINT_BASE, 1, ProbeWidth::B4);
    for _ in 0..50 {
        let _ = clint.tick();
    }

    let clint = restored(
        &clint,
        Clint::new(PhysAddr::new(CLINT_BASE), std::num::NonZero::new(1).unwrap(), 2),
    );

    assert_eq!(clint.mtime(), 50);
    assert!(clint.timer_pending(HartId::new(1)));
    assert!(!clint.timer_pending(HartId::new(0)));
    assert!(clint.msip_pending(HartId::new(0)));
}

#[test]
fn a_plic_keeps_its_priorities_enables_thresholds_and_claims() {
    let mut plic = Plic::new(PhysAddr::new(PLIC_BASE), 1);
    write(&mut plic, PLIC_BASE + 4 * 10, 5, ProbeWidth::B4);
    write(&mut plic, PLIC_BASE + 0x2000, 1 << 10, ProbeWidth::B4);
    write(&mut plic, PLIC_BASE + 0x20_0000, 2, ProbeWidth::B4);
    plic.update_irqs(1 << 10);
    for _ in 0..4 {
        plic.check_interrupts();
    }
    assert!(plic.hart_lines(HartId::new(0)).meip);

    let mut plic = restored(&plic, Plic::new(PhysAddr::new(PLIC_BASE), 1));

    assert!(plic.hart_lines(HartId::new(0)).meip, "the claim is visible at once");
    assert_eq!(read(&mut plic, PLIC_BASE + 4 * 10, ProbeWidth::B4), 5);
    assert_eq!(read(&mut plic, PLIC_BASE + 0x20_0004, ProbeWidth::B4), 10, "the claim register");
}

#[test]
fn a_uart_keeps_its_registers() {
    let mut uart = Uart::new(PhysAddr::new(UART_BASE), Terminal::shared(Console::Quiet), 2400);
    write(&mut uart, UART_BASE + 1, 0x01, ProbeWidth::B1);
    write(&mut uart, UART_BASE + 3, 0x03, ProbeWidth::B1);
    write(&mut uart, UART_BASE + 7, 0x5a, ProbeWidth::B1);

    let mut uart = restored(
        &uart,
        Uart::new(PhysAddr::new(UART_BASE), Terminal::shared(Console::Quiet), 2400),
    );

    assert_eq!(read(&mut uart, UART_BASE + 1, ProbeWidth::B1), 0x01);
    assert_eq!(read(&mut uart, UART_BASE + 3, ProbeWidth::B1), 0x03);
    assert_eq!(read(&mut uart, UART_BASE + 7, ProbeWidth::B1), 0x5a);
}

#[test]
fn a_virtio_disk_keeps_its_queue_configuration() {
    let mut disk = VirtioBlock::new(PhysAddr::new(VIRTIO_BASE));
    write(&mut disk, VIRTIO_BASE + 0x38, 8, ProbeWidth::B4);
    write(&mut disk, VIRTIO_BASE + 0x80, 0x8000_1000, ProbeWidth::B4);
    write(&mut disk, VIRTIO_BASE + 0x70, 0x0f, ProbeWidth::B4);

    let mut disk = restored(&disk, VirtioBlock::new(PhysAddr::new(VIRTIO_BASE)));

    assert_eq!(disk.state().queue_num, 8);
    assert_eq!(disk.state().queue_desc_low, 0x8000_1000);
    assert_eq!(read(&mut disk, VIRTIO_BASE + 0x70, ProbeWidth::B4), 0x0f);
}

#[test]
fn a_bus_checkpoint_restores_every_device_into_a_fresh_system() {
    let mut config = Config::minimal();
    config.system.hart_count = std::num::NonZero::new(2).unwrap();
    config.system.console = Console::Quiet;
    let mut sim = build_simulator(&config);
    sim.probe_mem_store(PhysAddr::new(CLINT_BASE + 0x4008), 1234, ProbeWidth::B8);
    sim.probe_mem_store(PhysAddr::new(PLIC_BASE + 4 * 10), 3, ProbeWidth::B4);
    for _ in 0..8 {
        sim.tick().unwrap();
    }
    let before = sim.state.uncore.bus.checkpoint_devices();

    let mut fresh = build_simulator(&config);
    fresh
        .state
        .uncore
        .bus
        .restore_devices(&before)
        .expect("a fresh system takes every device's state");

    assert_eq!(fresh.state.uncore.bus.checkpoint_devices(), before);
    assert_eq!(fresh.probe_mem_load(PhysAddr::new(CLINT_BASE + 0x4008), ProbeWidth::B8), 1234);
    assert_eq!(fresh.probe_mem_load(PhysAddr::new(PLIC_BASE + 4 * 10), ProbeWidth::B4), 3);
}
