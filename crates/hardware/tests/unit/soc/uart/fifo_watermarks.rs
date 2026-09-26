//! UART unit tests.
//!
//! Tests register read/write, LSR status, DLAB mode, scratch register,
//! and IER configuration. Note: we can't easily test stdin integration
//! in unit tests, so we focus on register-level behaviour.

use rvsim_core::common::IrqId;
use rvsim_core::soc::devices::Device;
use rvsim_core::soc::devices::uart::Uart;

#[test]
fn uart_name() {
    let uart = Uart::new(0x1000_0000, true, true, 2400);
    assert_eq!(uart.name(), "UART0");
}

#[test]
fn uart_address_range() {
    let uart = Uart::new(0x1000_0000, true, true, 2400);
    let (base, size) = uart.address_range();
    assert_eq!(base, 0x1000_0000);
    assert_eq!(size, 0x100);
}

#[test]
fn uart_lsr_default_thre_temt() {
    let mut uart = Uart::new(0, true, true, 2400);
    let lsr = crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(5), 1) as u8; // LSR
    // Bits 5 (THRE) and 6 (TEMT) should be set (transmitter ready)
    assert_ne!(lsr & 0x20, 0, "THRE should be set");
    assert_ne!(lsr & 0x40, 0, "TEMT should be set");
}

#[test]
fn uart_lsr_no_data_ready() {
    let mut uart = Uart::new(0, true, true, 2400);
    let lsr = crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(5), 1) as u8;
    assert_eq!(lsr & 0x01, 0, "No data ready initially");
}

#[test]
fn uart_scratch_register() {
    let mut uart = Uart::new(0, true, true, 2400);
    crate::common::probe::write(&mut uart, rvsim_core::common::PhysAddr::new(7), 0xAB_u64, 1); // SCR
    assert_eq!(
        (crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(7), 1) as u8),
        0xAB
    );
}

#[test]
fn uart_lcr_write_and_read() {
    let mut uart = Uart::new(0, true, true, 2400);
    crate::common::probe::write(&mut uart, rvsim_core::common::PhysAddr::new(3), 0x03_u64, 1); // LCR = 8N1
    assert_eq!(
        (crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(3), 1) as u8),
        0x03
    );
}

#[test]
fn uart_mcr_write_and_read() {
    let mut uart = Uart::new(0, true, true, 2400);
    crate::common::probe::write(&mut uart, rvsim_core::common::PhysAddr::new(4), 0x0B_u64, 1); // MCR
    assert_eq!(
        (crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(4), 1) as u8),
        0x0B
    );
}

#[test]
fn uart_dlab_mode_divisor() {
    let mut uart = Uart::new(0, true, true, 2400);
    // Set DLAB
    crate::common::probe::write(&mut uart, rvsim_core::common::PhysAddr::new(3), 0x80_u64, 1);
    // Write divisor latch low
    crate::common::probe::write(&mut uart, rvsim_core::common::PhysAddr::new(0), 0x01_u64, 1);
    // Write divisor latch high
    crate::common::probe::write(&mut uart, rvsim_core::common::PhysAddr::new(1), 0x00_u64, 1);
    // Read back DLL
    assert_eq!(
        (crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(0), 1) as u8),
        0x01
    );
    // Read back DLM
    assert_eq!(
        (crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(1), 1) as u8),
        0x00
    );
}

#[test]
fn uart_ier_write_and_read() {
    let mut uart = Uart::new(0, true, true, 2400);
    // Ensure DLAB is clear
    crate::common::probe::write(&mut uart, rvsim_core::common::PhysAddr::new(3), 0x00_u64, 1);
    crate::common::probe::write(&mut uart, rvsim_core::common::PhysAddr::new(1), 0x03_u64, 1); // IER: enable RDA and THRE interrupts
    assert_eq!(
        (crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(1), 1) as u8),
        0x03
    );
}

#[test]
fn uart_iir_no_interrupt_initially() {
    let mut uart = Uart::new(0, true, true, 2400);
    let iir = crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(2), 1) as u8;
    // 16550: IIR bit 0 = 1 means "no interrupt pending".
    assert_ne!(iir & 0x01, 0, "No interrupt pending initially");
}

#[test]
fn uart_irq_id() {
    let uart = Uart::new(0, true, true, 2400);
    assert_eq!(uart.get_irq_id(), Some(IrqId::new(10)));
}

#[test]
fn uart_msr_returns_zero() {
    let mut uart = Uart::new(0, true, true, 2400);
    assert_eq!(
        (crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(6), 1) as u8),
        0,
        "MSR should return 0"
    );
}

#[test]
fn uart_unknown_register_returns_zero() {
    let mut uart = Uart::new(0, true, true, 2400);
    // Register offset > 7 should return 0
    assert_eq!(
        (crate::common::probe::read(&mut uart, rvsim_core::common::PhysAddr::new(15), 1) as u8),
        0
    );
}
