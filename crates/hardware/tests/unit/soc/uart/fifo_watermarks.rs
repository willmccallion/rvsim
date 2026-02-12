//! UART unit tests.
//!
//! Tests register read/write, LSR status, DLAB mode, scratch register,
//! and IER configuration. Note: we can't easily test stdin integration
//! in unit tests, so we focus on register-level behaviour.

use riscv_core::soc::devices::Device;
use riscv_core::soc::devices::uart::Uart;

#[test]
fn uart_name() {
    let uart = Uart::new(0x1000_0000, true);
    assert_eq!(uart.name(), "UART0");
}

#[test]
fn uart_address_range() {
    let uart = Uart::new(0x1000_0000, true);
    let (base, size) = uart.address_range();
    assert_eq!(base, 0x1000_0000);
    assert_eq!(size, 0x100);
}

#[test]
fn uart_lsr_default_thre_temt() {
    let mut uart = Uart::new(0, true);
    let lsr = uart.read_u8(5); // LSR
    // Bits 5 (THRE) and 6 (TEMT) should be set (transmitter ready)
    assert_ne!(lsr & 0x20, 0, "THRE should be set");
    assert_ne!(lsr & 0x40, 0, "TEMT should be set");
}

#[test]
fn uart_lsr_no_data_ready() {
    let mut uart = Uart::new(0, true);
    let lsr = uart.read_u8(5);
    assert_eq!(lsr & 0x01, 0, "No data ready initially");
}

#[test]
fn uart_scratch_register() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(7, 0xAB); // SCR
    assert_eq!(uart.read_u8(7), 0xAB);
}

#[test]
fn uart_lcr_write_and_read() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(3, 0x03); // LCR = 8N1
    assert_eq!(uart.read_u8(3), 0x03);
}

#[test]
fn uart_mcr_write_and_read() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(4, 0x0B); // MCR
    assert_eq!(uart.read_u8(4), 0x0B);
}

#[test]
fn uart_dlab_mode_divisor() {
    let mut uart = Uart::new(0, true);
    // Set DLAB
    uart.write_u8(3, 0x80);
    // Write divisor latch low
    uart.write_u8(0, 0x01);
    // Write divisor latch high
    uart.write_u8(1, 0x00);
    // Read back DLL
    assert_eq!(uart.read_u8(0), 0x01);
    // Read back DLM
    assert_eq!(uart.read_u8(1), 0x00);
}

#[test]
fn uart_ier_write_and_read() {
    let mut uart = Uart::new(0, true);
    // Ensure DLAB is clear
    uart.write_u8(3, 0x00);
    uart.write_u8(1, 0x03); // IER: enable RDA and THRE interrupts
    assert_eq!(uart.read_u8(1), 0x03);
}

#[test]
fn uart_iir_no_interrupt_initially() {
    let mut uart = Uart::new(0, true);
    let iir = uart.read_u8(2);
    // Bit 0 should be 1 (no interrupt pending) per 16550 spec
    // IIR has bits 7:6 set, plus the ID
    assert_ne!(iir & 0x01, 0, "No interrupt pending initially");
}

#[test]
fn uart_irq_id() {
    let uart = Uart::new(0, true);
    assert_eq!(uart.get_irq_id(), Some(10));
}

#[test]
fn uart_msr_returns_zero() {
    let mut uart = Uart::new(0, true);
    assert_eq!(uart.read_u8(6), 0, "MSR should return 0");
}

#[test]
fn uart_unknown_register_returns_zero() {
    let mut uart = Uart::new(0, true);
    // Register offset > 7 should return 0
    assert_eq!(uart.read_u8(15), 0);
}
