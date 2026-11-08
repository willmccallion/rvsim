//! Comprehensive UART Tests.
//!
//! Tests for UART data transmission, receive buffer, interrupt handling,
//! and various register configurations.

use crate::system::simulator::ProbeWidth;

use crate::common::{IrqId, PhysAddr};
use crate::config::Console;
use crate::soc::devices::Device;
use crate::soc::devices::uart::Uart;
use crate::soc::terminal::{SharedTerminal, Terminal};
use crate::tests::support::probe;

fn uart(base_addr: u64, console: Console, cpu_clock_mhz: u64) -> Uart {
    Uart::new(PhysAddr::new(base_addr), Terminal::shared(console), cpu_clock_mhz)
}

#[test]
fn uart_dlab_set() {
    let mut uart = uart(0, Console::Quiet, 2400);
    // Set DLAB bit (bit 7 of LCR)
    probe::write(&mut uart, PhysAddr::new(3), 0x80_u64, ProbeWidth::B1);
    let lcr = probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8;
    assert_eq!(lcr & 0x80, 0x80);
}

#[test]
fn uart_dlab_divisor_low() {
    let mut uart = uart(0, Console::Quiet, 2400);
    // Enable DLAB
    probe::write(&mut uart, PhysAddr::new(3), 0x80_u64, ProbeWidth::B1);
    // Write divisor low byte
    probe::write(&mut uart, PhysAddr::new(0), 0x01_u64, ProbeWidth::B1);
    // Verify
    assert_eq!((probe::read(&mut uart, PhysAddr::new(0), ProbeWidth::B1) as u8), 0x01);
}

#[test]
fn uart_dlab_divisor_high() {
    let mut uart = uart(0, Console::Quiet, 2400);
    // Enable DLAB
    probe::write(&mut uart, PhysAddr::new(3), 0x80_u64, ProbeWidth::B1);
    // Write divisor high byte
    probe::write(&mut uart, PhysAddr::new(1), 0x00_u64, ProbeWidth::B1);
    // Verify
    assert_eq!((probe::read(&mut uart, PhysAddr::new(1), ProbeWidth::B1) as u8), 0x00);
}

#[test]
fn uart_dlab_full_divisor() {
    let mut uart = uart(0, Console::Quiet, 2400);
    // Set DLAB
    probe::write(&mut uart, PhysAddr::new(3), 0x80_u64, ProbeWidth::B1);
    // Set divisor = 0x000C (common for 9600 baud)
    probe::write(&mut uart, PhysAddr::new(0), 0x0C_u64, ProbeWidth::B1); // DLL
    probe::write(&mut uart, PhysAddr::new(1), 0x00_u64, ProbeWidth::B1); // DLH

    assert_eq!((probe::read(&mut uart, PhysAddr::new(0), ProbeWidth::B1) as u8), 0x0C);
    assert_eq!((probe::read(&mut uart, PhysAddr::new(1), ProbeWidth::B1) as u8), 0x00);
}

#[test]
fn uart_dlab_disable() {
    let mut uart = uart(0, Console::Quiet, 2400);
    // Enable DLAB
    probe::write(&mut uart, PhysAddr::new(3), 0x80_u64, ProbeWidth::B1);
    // Disable DLAB
    probe::write(&mut uart, PhysAddr::new(3), 0x03_u64, ProbeWidth::B1); // 8N1, no DLAB
    let lcr = probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8;
    assert_eq!(lcr & 0x80, 0);
}

#[test]
fn uart_lcr_data_bits_5() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(3), 0x00_u64, ProbeWidth::B1); // 5 data bits
    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8) & 0x03, 0x00);
}

#[test]
fn uart_lcr_data_bits_6() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(3), 0x01_u64, ProbeWidth::B1); // 6 data bits
    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8) & 0x03, 0x01);
}

#[test]
fn uart_lcr_data_bits_7() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(3), 0x02_u64, ProbeWidth::B1); // 7 data bits
    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8) & 0x03, 0x02);
}

#[test]
fn uart_lcr_data_bits_8() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(3), 0x03_u64, ProbeWidth::B1); // 8 data bits
    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8) & 0x03, 0x03);
}

#[test]
fn uart_lcr_stop_bits() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(3), 0x04_u64, ProbeWidth::B1); // 2 stop bits
    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8) & 0x04, 0x04);
}

#[test]
fn uart_lcr_parity_enable() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(3), 0x08_u64, ProbeWidth::B1); // Enable parity
    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8) & 0x08, 0x08);
}

#[test]
fn uart_lcr_even_parity() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(3), 0x18_u64, ProbeWidth::B1); // Enable parity + even parity
    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8) & 0x18, 0x18);
}

#[test]
fn uart_lcr_break_control() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(3), 0x40_u64, ProbeWidth::B1); // Set break
    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8) & 0x40, 0x40);
}

#[test]
fn uart_ier_disable_all() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(1), 0x00_u64, ProbeWidth::B1);
    assert_eq!((probe::read(&mut uart, PhysAddr::new(1), ProbeWidth::B1) as u8), 0x00);
}

#[test]
fn uart_ier_enable_received_data() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(1), 0x01_u64, ProbeWidth::B1); // Enable received data interrupt
    assert_eq!((probe::read(&mut uart, PhysAddr::new(1), ProbeWidth::B1) as u8), 0x01);
}

#[test]
fn uart_ier_enable_transmitter_empty() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(1), 0x02_u64, ProbeWidth::B1); // Enable transmitter empty interrupt
    assert_eq!((probe::read(&mut uart, PhysAddr::new(1), ProbeWidth::B1) as u8), 0x02);
}

#[test]
fn uart_ier_enable_line_status() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(1), 0x04_u64, ProbeWidth::B1); // Enable line status interrupt
    assert_eq!((probe::read(&mut uart, PhysAddr::new(1), ProbeWidth::B1) as u8), 0x04);
}

#[test]
fn uart_ier_enable_modem_status() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(1), 0x08_u64, ProbeWidth::B1); // Enable modem status interrupt
    assert_eq!((probe::read(&mut uart, PhysAddr::new(1), ProbeWidth::B1) as u8), 0x08);
}

#[test]
fn uart_ier_enable_multiple() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(1), 0x0F_u64, ProbeWidth::B1); // Enable all interrupts
    assert_eq!((probe::read(&mut uart, PhysAddr::new(1), ProbeWidth::B1) as u8), 0x0F);
}

#[test]
fn uart_iir_no_interrupt_pending() {
    let mut uart = uart(0, Console::Quiet, 2400);
    let iir = probe::read(&mut uart, PhysAddr::new(2), ProbeWidth::B1) as u8;
    // Bit 0 = 1 means no interrupt pending
    assert_ne!(iir & 0x01, 0);
}

#[test]
fn uart_mcr_dtr() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(4), 0x01_u64, ProbeWidth::B1); // DTR
    assert_eq!((probe::read(&mut uart, PhysAddr::new(4), ProbeWidth::B1) as u8) & 0x01, 0x01);
}

#[test]
fn uart_mcr_rts() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(4), 0x02_u64, ProbeWidth::B1); // RTS
    assert_eq!((probe::read(&mut uart, PhysAddr::new(4), ProbeWidth::B1) as u8) & 0x02, 0x02);
}

#[test]
fn uart_mcr_out1() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(4), 0x04_u64, ProbeWidth::B1); // OUT1
    assert_eq!((probe::read(&mut uart, PhysAddr::new(4), ProbeWidth::B1) as u8) & 0x04, 0x04);
}

#[test]
fn uart_mcr_out2() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(4), 0x08_u64, ProbeWidth::B1); // OUT2
    assert_eq!((probe::read(&mut uart, PhysAddr::new(4), ProbeWidth::B1) as u8) & 0x08, 0x08);
}

#[test]
fn uart_mcr_loopback() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(4), 0x10_u64, ProbeWidth::B1); // Loopback mode
    assert_eq!((probe::read(&mut uart, PhysAddr::new(4), ProbeWidth::B1) as u8) & 0x10, 0x10);
}

#[test]
fn uart_mcr_all_bits() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(4), 0x1F_u64, ProbeWidth::B1); // All bits set
    assert_eq!((probe::read(&mut uart, PhysAddr::new(4), ProbeWidth::B1) as u8), 0x1F);
}

#[test]
fn uart_irq_id() {
    let uart = uart(0x1000_0000, Console::Quiet, 2400);
    assert_eq!(uart.get_irq_id(), Some(IrqId::new(10)));
}

#[test]
fn uart_tick_no_interrupt() {
    let mut uart = uart(0, Console::Quiet, 2400);
    assert!(!uart.tick());
}

#[test]
fn uart_configure_9600_8n1() {
    let mut uart = uart(0, Console::Quiet, 2400);
    // Set DLAB
    probe::write(&mut uart, PhysAddr::new(3), 0x80_u64, ProbeWidth::B1);
    // Set divisor for 9600 baud
    probe::write(&mut uart, PhysAddr::new(0), 0x0C_u64, ProbeWidth::B1);
    probe::write(&mut uart, PhysAddr::new(1), 0x00_u64, ProbeWidth::B1);
    // Clear DLAB, set 8N1
    probe::write(&mut uart, PhysAddr::new(3), 0x03_u64, ProbeWidth::B1);

    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8) & 0x03, 0x03);
}

#[test]
fn uart_configure_115200_8n1() {
    let mut uart = uart(0, Console::Quiet, 2400);
    // Set DLAB
    probe::write(&mut uart, PhysAddr::new(3), 0x80_u64, ProbeWidth::B1);
    // Set divisor for 115200 baud
    probe::write(&mut uart, PhysAddr::new(0), 0x01_u64, ProbeWidth::B1);
    probe::write(&mut uart, PhysAddr::new(1), 0x00_u64, ProbeWidth::B1);
    // Clear DLAB, set 8N1
    probe::write(&mut uart, PhysAddr::new(3), 0x03_u64, ProbeWidth::B1);

    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8) & 0x03, 0x03);
}

#[test]
fn uart_configure_with_parity() {
    let mut uart = uart(0, Console::Quiet, 2400);
    // 8 data bits, 1 stop bit, even parity
    probe::write(&mut uart, PhysAddr::new(3), 0x1B_u64, ProbeWidth::B1); // 0b00011011
    assert_eq!((probe::read(&mut uart, PhysAddr::new(3), ProbeWidth::B1) as u8), 0x1B);
}

#[test]
fn uart_configure_with_flow_control() {
    let mut uart = uart(0, Console::Quiet, 2400);
    // Set RTS/DTR for flow control
    probe::write(&mut uart, PhysAddr::new(4), 0x03_u64, ProbeWidth::B1);
    assert_eq!((probe::read(&mut uart, PhysAddr::new(4), ProbeWidth::B1) as u8) & 0x03, 0x03);
}

#[test]
fn uart_reset_configuration() {
    let mut uart = uart(0, Console::Quiet, 2400);
    // Set some configuration
    probe::write(&mut uart, PhysAddr::new(3), 0x03_u64, ProbeWidth::B1);
    probe::write(&mut uart, PhysAddr::new(4), 0x03_u64, ProbeWidth::B1);
    probe::write(&mut uart, PhysAddr::new(1), 0x0F_u64, ProbeWidth::B1);

    // Reset
    probe::write(&mut uart, PhysAddr::new(3), 0x00_u64, ProbeWidth::B1);
    probe::write(&mut uart, PhysAddr::new(4), 0x00_u64, ProbeWidth::B1);
    probe::write(&mut uart, PhysAddr::new(1), 0x00_u64, ProbeWidth::B1);

    assert_eq!((probe::read(&mut uart, PhysAddr::new(1), ProbeWidth::B1) as u8), 0x00);
}

#[test]
fn uart_scratch_all_zeros() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(7), 0x00_u64, ProbeWidth::B1);
    assert_eq!((probe::read(&mut uart, PhysAddr::new(7), ProbeWidth::B1) as u8), 0x00);
}

#[test]
fn uart_scratch_all_ones() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(7), 0xFF_u64, ProbeWidth::B1);
    assert_eq!((probe::read(&mut uart, PhysAddr::new(7), ProbeWidth::B1) as u8), 0xFF);
}

#[test]
fn uart_scratch_pattern() {
    let mut uart = uart(0, Console::Quiet, 2400);
    probe::write(&mut uart, PhysAddr::new(7), 0x55_u64, ProbeWidth::B1);
    assert_eq!((probe::read(&mut uart, PhysAddr::new(7), ProbeWidth::B1) as u8), 0x55);

    probe::write(&mut uart, PhysAddr::new(7), 0xAA_u64, ProbeWidth::B1);
    assert_eq!((probe::read(&mut uart, PhysAddr::new(7), ProbeWidth::B1) as u8), 0xAA);
}

/// Seeing the panic message is the simulator's business; the guest's
/// console still prints every byte of it.
#[test]
fn a_kernel_panic_message_is_printed_whole_and_noticed() {
    let terminal: SharedTerminal = Terminal::shared(Console::Captured);
    let mut uart = Uart::new(PhysAddr::new(0), terminal.clone(), 2400);
    for &byte in b"Kernel panic - not syncing" {
        probe::write(&mut uart, PhysAddr::new(0), u64::from(byte), ProbeWidth::B1);
    }

    let output = terminal.lock().expect("the terminal is free").take_output();
    assert_eq!(
        (output.as_slice(), uart.check_kernel_panic()),
        (&b"Kernel panic - not syncing"[..], true)
    );
}

#[test]
fn a_captured_console_keeps_output_for_the_host() {
    let terminal: SharedTerminal = Terminal::shared(Console::Captured);
    let mut uart = Uart::new(PhysAddr::new(0), terminal.clone(), 2400);
    for &byte in b"ok\n" {
        probe::write(&mut uart, PhysAddr::new(0), u64::from(byte), ProbeWidth::B1);
    }

    assert_eq!(terminal.lock().expect("the terminal is free").take_output(), b"ok\n");
    assert!(
        terminal.lock().expect("the terminal is free").take_output().is_empty(),
        "each byte is taken once"
    );
}

#[test]
fn a_captured_console_delivers_the_hosts_input() {
    let terminal = Terminal::shared(Console::Captured);
    let mut uart = Uart::new(PhysAddr::new(0), terminal.clone(), 2400);

    terminal.lock().expect("the terminal is free").type_input(b"r");
    for _ in 0..=u8::MAX {
        let _ = uart.tick();
    }

    let lsr = probe::read(&mut uart, PhysAddr::new(5), ProbeWidth::B1);
    let rbr = probe::read(&mut uart, PhysAddr::new(0), ProbeWidth::B1);
    assert_eq!((lsr & 1, rbr), (1, u64::from(b'r')));
}

/// A UART at address 0 and the captured console it writes to.
fn captured_uart() -> (Uart, SharedTerminal) {
    let terminal: SharedTerminal = Terminal::shared(Console::Captured);
    (Uart::new(PhysAddr::new(0), terminal.clone(), 2400), terminal)
}

fn console_output(terminal: &SharedTerminal) -> Vec<u8> {
    terminal.lock().expect("the terminal is free").take_output()
}

#[test]
fn bytes_written_to_thr_reach_the_console_in_order() {
    let (mut uart, terminal) = captured_uart();

    for &byte in b"Hello" {
        probe::write(&mut uart, PhysAddr::new(0), u64::from(byte), ProbeWidth::B1);
    }

    assert_eq!(console_output(&terminal), b"Hello");
}

#[test]
fn a_wide_write_to_thr_sends_only_its_low_byte() {
    let (mut uart, terminal) = captured_uart();

    for width in [ProbeWidth::B2, ProbeWidth::B4, ProbeWidth::B8] {
        probe::write(&mut uart, PhysAddr::new(0), 0x4142_4344_4546_4748, width);
    }

    assert_eq!(console_output(&terminal), b"HHH");
}

#[test]
fn a_fresh_uart_has_an_empty_transmitter_and_no_line_errors() {
    let mut uart = uart(0, Console::Quiet, 2400);

    let lsr = probe::read(&mut uart, PhysAddr::new(5), ProbeWidth::B1);

    assert_eq!(lsr, 0x60, "THRE and TEMT, with no overrun, parity, framing or break bit");
}

#[test]
fn a_fresh_uart_reports_its_fifos_and_no_pending_interrupt() {
    let mut uart = uart(0, Console::Quiet, 2400);

    let iir = probe::read(&mut uart, PhysAddr::new(2), ProbeWidth::B1);

    assert_eq!(iir, 0xC1);
}

#[test]
fn a_wide_read_returns_the_register_byte() {
    let mut uart = uart(0, Console::Quiet, 2400);

    let reads = [ProbeWidth::B2, ProbeWidth::B4, ProbeWidth::B8]
        .map(|width| probe::read(&mut uart, PhysAddr::new(5), width));

    assert_eq!(reads, [0x60; 3]);
}

#[test]
fn an_unknown_register_reads_zero_and_ignores_writes() {
    let mut uart = uart(0, Console::Quiet, 2400);

    probe::write(&mut uart, PhysAddr::new(0xFF), 0xAB, ProbeWidth::B1);

    assert_eq!(probe::read(&mut uart, PhysAddr::new(0xFF), ProbeWidth::B1), 0);
}
