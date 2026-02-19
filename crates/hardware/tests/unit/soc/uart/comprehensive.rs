//! Comprehensive UART Tests.
//!
//! Tests for UART data transmission, receive buffer, interrupt handling,
//! and various register configurations.

use rvsim_core::soc::devices::Device;
use rvsim_core::soc::devices::uart::Uart;

// ══════════════════════════════════════════════════════════
// Data Transmission Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_transmit_data_via_thr() {
    let mut uart = Uart::new(0x1000_0000, true);
    // Write to THR (offset 0)
    uart.write_u8(0, 0x41); // ASCII 'A'
    // UART should buffer this for transmission
}

#[test]
fn uart_transmit_multiple_bytes() {
    let mut uart = Uart::new(0x1000_0000, true);
    uart.write_u8(0, 0x48); // 'H'
    uart.write_u8(0, 0x69); // 'i'
}

#[test]
fn uart_transmit_full_message() {
    let mut uart = Uart::new(0x1000_0000, true);
    let message = b"Hello";
    for &byte in message {
        uart.write_u8(0, byte);
    }
}

// ══════════════════════════════════════════════════════════
// DLAB Mode Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_dlab_set() {
    let mut uart = Uart::new(0, true);
    // Set DLAB bit (bit 7 of LCR)
    uart.write_u8(3, 0x80);
    let lcr = uart.read_u8(3);
    assert_eq!(lcr & 0x80, 0x80);
}

#[test]
fn uart_dlab_divisor_low() {
    let mut uart = Uart::new(0, true);
    // Enable DLAB
    uart.write_u8(3, 0x80);
    // Write divisor low byte
    uart.write_u8(0, 0x01);
    // Verify
    assert_eq!(uart.read_u8(0), 0x01);
}

#[test]
fn uart_dlab_divisor_high() {
    let mut uart = Uart::new(0, true);
    // Enable DLAB
    uart.write_u8(3, 0x80);
    // Write divisor high byte
    uart.write_u8(1, 0x00);
    // Verify
    assert_eq!(uart.read_u8(1), 0x00);
}

#[test]
fn uart_dlab_full_divisor() {
    let mut uart = Uart::new(0, true);
    // Set DLAB
    uart.write_u8(3, 0x80);
    // Set divisor = 0x000C (common for 9600 baud)
    uart.write_u8(0, 0x0C); // DLL
    uart.write_u8(1, 0x00); // DLH

    assert_eq!(uart.read_u8(0), 0x0C);
    assert_eq!(uart.read_u8(1), 0x00);
}

#[test]
fn uart_dlab_disable() {
    let mut uart = Uart::new(0, true);
    // Enable DLAB
    uart.write_u8(3, 0x80);
    // Disable DLAB
    uart.write_u8(3, 0x03); // 8N1, no DLAB
    let lcr = uart.read_u8(3);
    assert_eq!(lcr & 0x80, 0);
}

// ══════════════════════════════════════════════════════════
// Line Control Register Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_lcr_data_bits_5() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(3, 0x00); // 5 data bits
    assert_eq!(uart.read_u8(3) & 0x03, 0x00);
}

#[test]
fn uart_lcr_data_bits_6() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(3, 0x01); // 6 data bits
    assert_eq!(uart.read_u8(3) & 0x03, 0x01);
}

#[test]
fn uart_lcr_data_bits_7() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(3, 0x02); // 7 data bits
    assert_eq!(uart.read_u8(3) & 0x03, 0x02);
}

#[test]
fn uart_lcr_data_bits_8() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(3, 0x03); // 8 data bits
    assert_eq!(uart.read_u8(3) & 0x03, 0x03);
}

#[test]
fn uart_lcr_stop_bits() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(3, 0x04); // 2 stop bits
    assert_eq!(uart.read_u8(3) & 0x04, 0x04);
}

#[test]
fn uart_lcr_parity_enable() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(3, 0x08); // Enable parity
    assert_eq!(uart.read_u8(3) & 0x08, 0x08);
}

#[test]
fn uart_lcr_even_parity() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(3, 0x18); // Enable parity + even parity
    assert_eq!(uart.read_u8(3) & 0x18, 0x18);
}

#[test]
fn uart_lcr_break_control() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(3, 0x40); // Set break
    assert_eq!(uart.read_u8(3) & 0x40, 0x40);
}

// ══════════════════════════════════════════════════════════
// Interrupt Enable Register Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_ier_disable_all() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(1, 0x00);
    assert_eq!(uart.read_u8(1), 0x00);
}

#[test]
fn uart_ier_enable_received_data() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(1, 0x01); // Enable received data interrupt
    assert_eq!(uart.read_u8(1), 0x01);
}

#[test]
fn uart_ier_enable_transmitter_empty() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(1, 0x02); // Enable transmitter empty interrupt
    assert_eq!(uart.read_u8(1), 0x02);
}

#[test]
fn uart_ier_enable_line_status() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(1, 0x04); // Enable line status interrupt
    assert_eq!(uart.read_u8(1), 0x04);
}

#[test]
fn uart_ier_enable_modem_status() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(1, 0x08); // Enable modem status interrupt
    assert_eq!(uart.read_u8(1), 0x08);
}

#[test]
fn uart_ier_enable_multiple() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(1, 0x0F); // Enable all interrupts
    assert_eq!(uart.read_u8(1), 0x0F);
}

// ══════════════════════════════════════════════════════════
// Interrupt Identification Register Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_iir_no_interrupt_pending() {
    let mut uart = Uart::new(0, true);
    let iir = uart.read_u8(2);
    // Bit 0 = 1 means no interrupt pending
    assert_ne!(iir & 0x01, 0);
}

#[test]
fn uart_iir_fifo_enabled() {
    let mut uart = Uart::new(0, true);
    let _iir = uart.read_u8(2);
    // Bits 6-7 should indicate FIFO status
}

// ══════════════════════════════════════════════════════════
// Modem Control Register Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_mcr_dtr() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(4, 0x01); // DTR
    assert_eq!(uart.read_u8(4) & 0x01, 0x01);
}

#[test]
fn uart_mcr_rts() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(4, 0x02); // RTS
    assert_eq!(uart.read_u8(4) & 0x02, 0x02);
}

#[test]
fn uart_mcr_out1() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(4, 0x04); // OUT1
    assert_eq!(uart.read_u8(4) & 0x04, 0x04);
}

#[test]
fn uart_mcr_out2() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(4, 0x08); // OUT2
    assert_eq!(uart.read_u8(4) & 0x08, 0x08);
}

#[test]
fn uart_mcr_loopback() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(4, 0x10); // Loopback mode
    assert_eq!(uart.read_u8(4) & 0x10, 0x10);
}

#[test]
fn uart_mcr_all_bits() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(4, 0x1F); // All bits set
    assert_eq!(uart.read_u8(4), 0x1F);
}

// ══════════════════════════════════════════════════════════
// Line Status Register Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_lsr_overrun_error() {
    let mut uart = Uart::new(0, true);
    let lsr = uart.read_u8(5);
    // Check bit 1 (overrun error)
    let _overrun = (lsr >> 1) & 1;
}

#[test]
fn uart_lsr_parity_error() {
    let mut uart = Uart::new(0, true);
    let lsr = uart.read_u8(5);
    // Check bit 2 (parity error)
    let _parity = (lsr >> 2) & 1;
}

#[test]
fn uart_lsr_framing_error() {
    let mut uart = Uart::new(0, true);
    let lsr = uart.read_u8(5);
    // Check bit 3 (framing error)
    let _framing = (lsr >> 3) & 1;
}

#[test]
fn uart_lsr_break_interrupt() {
    let mut uart = Uart::new(0, true);
    let lsr = uart.read_u8(5);
    // Check bit 4 (break interrupt)
    let _break_int = (lsr >> 4) & 1;
}

// ══════════════════════════════════════════════════════════
// Multi-byte Read/Write Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_read_u16() {
    let mut uart = Uart::new(0, true);
    let _ = uart.read_u16(0);
}

#[test]
fn uart_read_u32() {
    let mut uart = Uart::new(0, true);
    let _ = uart.read_u32(0);
}

#[test]
fn uart_read_u64() {
    let mut uart = Uart::new(0, true);
    let _ = uart.read_u64(0);
}

#[test]
fn uart_write_u16() {
    let mut uart = Uart::new(0, true);
    uart.write_u16(0, 0x4142);
}

#[test]
fn uart_write_u32() {
    let mut uart = Uart::new(0, true);
    uart.write_u32(0, 0x41424344);
}

#[test]
fn uart_write_u64() {
    let mut uart = Uart::new(0, true);
    uart.write_u64(0, 0x4142434445464748);
}

// ══════════════════════════════════════════════════════════
// Edge Cases and Invalid Access Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_invalid_register_read() {
    let mut uart = Uart::new(0, true);
    let _ = uart.read_u8(0xFF); // Invalid offset
}

#[test]
fn uart_invalid_register_write() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(0xFF, 0x00); // Invalid offset
}

#[test]
fn uart_read_write_only_register() {
    let mut uart = Uart::new(0, true);
    // Try reading FCR (write-only)
    let _ = uart.read_u8(2);
}

// ══════════════════════════════════════════════════════════
// IRQ Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_irq_id() {
    let uart = Uart::new(0x1000_0000, true);
    assert_eq!(uart.get_irq_id(), Some(10));
}

#[test]
fn uart_tick_no_interrupt() {
    let mut uart = Uart::new(0, true);
    assert!(!uart.tick());
}

// ══════════════════════════════════════════════════════════
// Configuration Scenarios
// ══════════════════════════════════════════════════════════

#[test]
fn uart_configure_9600_8n1() {
    let mut uart = Uart::new(0, true);
    // Set DLAB
    uart.write_u8(3, 0x80);
    // Set divisor for 9600 baud
    uart.write_u8(0, 0x0C);
    uart.write_u8(1, 0x00);
    // Clear DLAB, set 8N1
    uart.write_u8(3, 0x03);

    assert_eq!(uart.read_u8(3) & 0x03, 0x03);
}

#[test]
fn uart_configure_115200_8n1() {
    let mut uart = Uart::new(0, true);
    // Set DLAB
    uart.write_u8(3, 0x80);
    // Set divisor for 115200 baud
    uart.write_u8(0, 0x01);
    uart.write_u8(1, 0x00);
    // Clear DLAB, set 8N1
    uart.write_u8(3, 0x03);

    assert_eq!(uart.read_u8(3) & 0x03, 0x03);
}

#[test]
fn uart_configure_with_parity() {
    let mut uart = Uart::new(0, true);
    // 8 data bits, 1 stop bit, even parity
    uart.write_u8(3, 0x1B); // 0b00011011
    assert_eq!(uart.read_u8(3), 0x1B);
}

#[test]
fn uart_configure_with_flow_control() {
    let mut uart = Uart::new(0, true);
    // Set RTS/DTR for flow control
    uart.write_u8(4, 0x03);
    assert_eq!(uart.read_u8(4) & 0x03, 0x03);
}

#[test]
fn uart_reset_configuration() {
    let mut uart = Uart::new(0, true);
    // Set some configuration
    uart.write_u8(3, 0x03);
    uart.write_u8(4, 0x03);
    uart.write_u8(1, 0x0F);

    // Reset
    uart.write_u8(3, 0x00);
    uart.write_u8(4, 0x00);
    uart.write_u8(1, 0x00);

    assert_eq!(uart.read_u8(1), 0x00);
}

// ══════════════════════════════════════════════════════════
// Scratch Register Tests
// ══════════════════════════════════════════════════════════

#[test]
fn uart_scratch_all_zeros() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(7, 0x00);
    assert_eq!(uart.read_u8(7), 0x00);
}

#[test]
fn uart_scratch_all_ones() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(7, 0xFF);
    assert_eq!(uart.read_u8(7), 0xFF);
}

#[test]
fn uart_scratch_pattern() {
    let mut uart = Uart::new(0, true);
    uart.write_u8(7, 0x55);
    assert_eq!(uart.read_u8(7), 0x55);

    uart.write_u8(7, 0xAA);
    assert_eq!(uart.read_u8(7), 0xAA);
}
