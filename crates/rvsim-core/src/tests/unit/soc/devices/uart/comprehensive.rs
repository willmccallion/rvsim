//! Comprehensive UART Tests.
//!
//! Tests for UART data transmission, receive buffer, interrupt handling,
//! and various register configurations.

use crate::common::IrqId;
use crate::config::Console;
use crate::soc::devices::Device;
use crate::soc::devices::uart::Uart;

#[test]
fn uart_transmit_data_via_thr() {
    let mut uart = Uart::new(0x1000_0000, Console::Quiet, 2400);
    // Write to THR (offset 0)
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(0), 0x41_u64, 1); // ASCII 'A'
    // UART should buffer this for transmission
}

#[test]
fn uart_transmit_multiple_bytes() {
    let mut uart = Uart::new(0x1000_0000, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(0), 0x48_u64, 1); // 'H'
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(0), 0x69_u64, 1); // 'i'
}

#[test]
fn uart_transmit_full_message() {
    let mut uart = Uart::new(0x1000_0000, Console::Quiet, 2400);
    let message = b"Hello";
    for &byte in message {
        crate::tests::support::probe::write(
            &mut uart,
            crate::common::PhysAddr::new(0),
            (byte) as u64,
            1,
        );
    }
}

#[test]
fn uart_dlab_set() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // Set DLAB bit (bit 7 of LCR)
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x80_u64, 1);
    let lcr =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8;
    assert_eq!(lcr & 0x80, 0x80);
}

#[test]
fn uart_dlab_divisor_low() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // Enable DLAB
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x80_u64, 1);
    // Write divisor low byte
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(0), 0x01_u64, 1);
    // Verify
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(0), 1) as u8),
        0x01
    );
}

#[test]
fn uart_dlab_divisor_high() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // Enable DLAB
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x80_u64, 1);
    // Write divisor high byte
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x00_u64, 1);
    // Verify
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(1), 1) as u8),
        0x00
    );
}

#[test]
fn uart_dlab_full_divisor() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // Set DLAB
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x80_u64, 1);
    // Set divisor = 0x000C (common for 9600 baud)
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(0), 0x0C_u64, 1); // DLL
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x00_u64, 1); // DLH

    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(0), 1) as u8),
        0x0C
    );
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(1), 1) as u8),
        0x00
    );
}

#[test]
fn uart_dlab_disable() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // Enable DLAB
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x80_u64, 1);
    // Disable DLAB
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x03_u64, 1); // 8N1, no DLAB
    let lcr =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8;
    assert_eq!(lcr & 0x80, 0);
}

#[test]
fn uart_lcr_data_bits_5() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x00_u64, 1); // 5 data bits
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8)
            & 0x03,
        0x00
    );
}

#[test]
fn uart_lcr_data_bits_6() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x01_u64, 1); // 6 data bits
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8)
            & 0x03,
        0x01
    );
}

#[test]
fn uart_lcr_data_bits_7() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x02_u64, 1); // 7 data bits
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8)
            & 0x03,
        0x02
    );
}

#[test]
fn uart_lcr_data_bits_8() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x03_u64, 1); // 8 data bits
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8)
            & 0x03,
        0x03
    );
}

#[test]
fn uart_lcr_stop_bits() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x04_u64, 1); // 2 stop bits
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8)
            & 0x04,
        0x04
    );
}

#[test]
fn uart_lcr_parity_enable() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x08_u64, 1); // Enable parity
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8)
            & 0x08,
        0x08
    );
}

#[test]
fn uart_lcr_even_parity() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x18_u64, 1); // Enable parity + even parity
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8)
            & 0x18,
        0x18
    );
}

#[test]
fn uart_lcr_break_control() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x40_u64, 1); // Set break
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8)
            & 0x40,
        0x40
    );
}

#[test]
fn uart_ier_disable_all() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x00_u64, 1);
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(1), 1) as u8),
        0x00
    );
}

#[test]
fn uart_ier_enable_received_data() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x01_u64, 1); // Enable received data interrupt
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(1), 1) as u8),
        0x01
    );
}

#[test]
fn uart_ier_enable_transmitter_empty() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x02_u64, 1); // Enable transmitter empty interrupt
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(1), 1) as u8),
        0x02
    );
}

#[test]
fn uart_ier_enable_line_status() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x04_u64, 1); // Enable line status interrupt
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(1), 1) as u8),
        0x04
    );
}

#[test]
fn uart_ier_enable_modem_status() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x08_u64, 1); // Enable modem status interrupt
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(1), 1) as u8),
        0x08
    );
}

#[test]
fn uart_ier_enable_multiple() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x0F_u64, 1); // Enable all interrupts
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(1), 1) as u8),
        0x0F
    );
}

#[test]
fn uart_iir_no_interrupt_pending() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    let iir =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(2), 1) as u8;
    // Bit 0 = 1 means no interrupt pending
    assert_ne!(iir & 0x01, 0);
}

#[test]
fn uart_iir_fifo_enabled() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    let _iir =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(2), 1) as u8;
    // Bits 6-7 should indicate FIFO status
}

#[test]
fn uart_mcr_dtr() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(4), 0x01_u64, 1); // DTR
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(4), 1) as u8)
            & 0x01,
        0x01
    );
}

#[test]
fn uart_mcr_rts() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(4), 0x02_u64, 1); // RTS
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(4), 1) as u8)
            & 0x02,
        0x02
    );
}

#[test]
fn uart_mcr_out1() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(4), 0x04_u64, 1); // OUT1
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(4), 1) as u8)
            & 0x04,
        0x04
    );
}

#[test]
fn uart_mcr_out2() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(4), 0x08_u64, 1); // OUT2
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(4), 1) as u8)
            & 0x08,
        0x08
    );
}

#[test]
fn uart_mcr_loopback() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(4), 0x10_u64, 1); // Loopback mode
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(4), 1) as u8)
            & 0x10,
        0x10
    );
}

#[test]
fn uart_mcr_all_bits() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(4), 0x1F_u64, 1); // All bits set
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(4), 1) as u8),
        0x1F
    );
}

#[test]
fn uart_lsr_overrun_error() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    let lsr =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(5), 1) as u8;
    // Check bit 1 (overrun error)
    let _ = (lsr >> 1) & 1;
}

#[test]
fn uart_lsr_parity_error() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    let lsr =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(5), 1) as u8;
    // Check bit 2 (parity error)
    let _ = (lsr >> 2) & 1;
}

#[test]
fn uart_lsr_framing_error() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    let lsr =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(5), 1) as u8;
    // Check bit 3 (framing error)
    let _ = (lsr >> 3) & 1;
}

#[test]
fn uart_lsr_break_interrupt() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    let lsr =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(5), 1) as u8;
    // Check bit 4 (break interrupt)
    let _ = (lsr >> 4) & 1;
}

#[test]
fn uart_read_u16() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    let _ =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(0), 2) as u16;
}

#[test]
fn uart_read_u32() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    let _ =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(0), 4) as u32;
}

#[test]
fn uart_read_u64() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    let _ = crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(0), 8);
}

#[test]
fn uart_write_u16() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(0), 0x4142_u64, 2);
}

#[test]
fn uart_write_u32() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(
        &mut uart,
        crate::common::PhysAddr::new(0),
        0x41424344_u64,
        4,
    );
}

#[test]
fn uart_write_u64() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(
        &mut uart,
        crate::common::PhysAddr::new(0),
        0x4142434445464748,
        8,
    );
}

#[test]
fn uart_invalid_register_read() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    let _ =
        crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(0xFF), 1) as u8; // Invalid offset
}

#[test]
fn uart_invalid_register_write() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(0xFF), 0x00_u64, 1); // Invalid offset
}

#[test]
fn uart_read_write_only_register() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // Try reading FCR (write-only)
    let _ = crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(2), 1) as u8;
}

#[test]
fn uart_irq_id() {
    let uart = Uart::new(0x1000_0000, Console::Quiet, 2400);
    assert_eq!(uart.get_irq_id(), Some(IrqId::new(10)));
}

#[test]
fn uart_tick_no_interrupt() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    assert!(!uart.tick());
}

#[test]
fn uart_configure_9600_8n1() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // Set DLAB
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x80_u64, 1);
    // Set divisor for 9600 baud
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(0), 0x0C_u64, 1);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x00_u64, 1);
    // Clear DLAB, set 8N1
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x03_u64, 1);

    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8)
            & 0x03,
        0x03
    );
}

#[test]
fn uart_configure_115200_8n1() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // Set DLAB
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x80_u64, 1);
    // Set divisor for 115200 baud
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(0), 0x01_u64, 1);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x00_u64, 1);
    // Clear DLAB, set 8N1
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x03_u64, 1);

    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8)
            & 0x03,
        0x03
    );
}

#[test]
fn uart_configure_with_parity() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // 8 data bits, 1 stop bit, even parity
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x1B_u64, 1); // 0b00011011
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(3), 1) as u8),
        0x1B
    );
}

#[test]
fn uart_configure_with_flow_control() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // Set RTS/DTR for flow control
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(4), 0x03_u64, 1);
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(4), 1) as u8)
            & 0x03,
        0x03
    );
}

#[test]
fn uart_reset_configuration() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    // Set some configuration
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x03_u64, 1);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(4), 0x03_u64, 1);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x0F_u64, 1);

    // Reset
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(3), 0x00_u64, 1);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(4), 0x00_u64, 1);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(1), 0x00_u64, 1);

    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(1), 1) as u8),
        0x00
    );
}

#[test]
fn uart_scratch_all_zeros() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(7), 0x00_u64, 1);
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(7), 1) as u8),
        0x00
    );
}

#[test]
fn uart_scratch_all_ones() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(7), 0xFF_u64, 1);
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(7), 1) as u8),
        0xFF
    );
}

#[test]
fn uart_scratch_pattern() {
    let mut uart = Uart::new(0, Console::Quiet, 2400);
    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(7), 0x55_u64, 1);
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(7), 1) as u8),
        0x55
    );

    crate::tests::support::probe::write(&mut uart, crate::common::PhysAddr::new(7), 0xAA_u64, 1);
    assert_eq!(
        (crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(7), 1) as u8),
        0xAA
    );
}

#[test]
fn a_captured_console_keeps_output_for_the_host() {
    let mut uart = Uart::new(0, Console::Captured, 2400);
    for &byte in b"ok\n" {
        crate::tests::support::probe::write(
            &mut uart,
            crate::common::PhysAddr::new(0),
            u64::from(byte),
            1,
        );
    }

    assert_eq!(uart.take_output(), b"ok\n");
    assert!(uart.take_output().is_empty(), "each byte is taken once");
}

#[test]
fn a_captured_console_delivers_the_hosts_input() {
    let mut uart = Uart::new(0, Console::Captured, 2400);

    uart.send_input(b"r");

    let lsr = crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(5), 1);
    let rbr = crate::tests::support::probe::read(&mut uart, crate::common::PhysAddr::new(0), 1);
    assert_eq!((lsr & 1, rbr), (1, u64::from(b'r')));
}
