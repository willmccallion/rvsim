//! Universal Asynchronous Receiver-Transmitter (UART).
//!
//! Implements a 16550-compatible UART device for serial communication.
//! Handles standard registers (RBR, THR, IER, IIR, LCR, LSR) and integrates
//! with stdin/stdout for console I/O.

use crate::soc::devices::Device;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};
use std::thread;

/// Receiver Buffer Register (Read) / Divisor Latch Low (DLAB=1).
const REG_RBR: u64 = 0;
/// Transmitter Holding Register (Write) / Divisor Latch Low (DLAB=1).
const REG_THR: u64 = 0;
/// Interrupt Enable Register / Divisor Latch High (DLAB=1).
const REG_IER: u64 = 1;
/// Interrupt Identity Register (Read).
const REG_IIR: u64 = 2;
/// FIFO Control Register (Write).
const REG_FCR: u64 = 2;
/// Line Control Register.
const REG_LCR: u64 = 3;
/// Modem Control Register.
const REG_MCR: u64 = 4;
/// Line Status Register.
const REG_LSR: u64 = 5;
/// Modem Status Register.
const REG_MSR: u64 = 6;
/// Scratch Register.
const REG_SCR: u64 = 7;

/// Interrupt Identity Register: No interrupt pending.
const IIR_NO_INTERRUPT: u8 = 0x01;

/// Interrupt Identity Register: Transmitter Holding Register Empty interrupt.
const IIR_THRE: u8 = 0x02;

/// Interrupt Identity Register: Receiver Data Available interrupt.
const IIR_RDA: u8 = 0x04;

/// Interrupt Identity Register: Interrupt ID mask (bits 7:6).
const IIR_ID_MASK: u8 = 0xC0;

/// Line Status Register: Data ready bit (receiver has data).
const LSR_DATA_READY: u8 = 0x01;

/// Line Status Register: Transmitter Holding Register Empty.
const LSR_THRE: u8 = 0x20;

/// Line Status Register: Transmitter Empty (both THR and shift register empty).
const LSR_TEMT: u8 = 0x40;

/// Default Line Status Register value (transmitter ready).
const LSR_DEFAULT: u8 = LSR_THRE | LSR_TEMT;

/// Line Control Register: Divisor Latch Access Bit (enables baud rate programming).
const LCR_DLAB: u8 = 0x80;

/// Interrupt Enable Register: Receiver Data Available interrupt enable.
const IER_RDA: u8 = 0x01;

/// Interrupt Enable Register: Transmitter Holding Register Empty interrupt enable.
const IER_THRE: u8 = 0x02;

/// Threshold for flushing transmit buffer to stdout (4 KiB).
const TX_BUFFER_FLUSH_THRESHOLD: usize = 4096;

/// UART device structure.
///
/// Simulates a 16550 UART. It spawns a background thread to capture `stdin`
/// for input and writes output directly to `stdout`.
pub struct Uart {
    /// Base physical address of the device.
    base_addr: u64,
    /// Queue for received bytes (from stdin).
    rx_queue: VecDeque<u8>,
    /// Channel receiver for stdin thread. Wrapped in Mutex for Sync.
    rx_receiver: Mutex<Receiver<u8>>,
    /// Interrupt Enable Register.
    ier: u8,
    /// Line Control Register.
    lcr: u8,
    /// Modem Control Register.
    mcr: u8,
    /// Scratch Register.
    scr: u8,
    /// Divisor Latch (Baud Rate).
    div: u16,
    /// Internal tick counter for polling stdin.
    tick_count: u8,
    /// Transmitter Holding Register Empty Interrupt Pending.
    thre_ip: bool,
    /// Buffer for outgoing bytes (to stdout or stderr).
    tx_buffer: Vec<u8>,
    /// When true, output goes to stderr (for visibility when run from Python).
    to_stderr: bool,
    /// State machine index for panic detection.
    panic_match_state: usize,
    /// Flag indicating if a kernel panic string was detected.
    panic_detected: bool,
}

impl Uart {
    /// Creates a new UART device.
    ///
    /// Spawns a background thread to read from stdin.
    ///
    /// # Arguments
    ///
    /// * `base_addr` - The base physical address of the UART device.
    /// * `to_stderr` - When true, write output to stderr instead of stdout (for Python API).
    pub fn new(base_addr: u64, to_stderr: bool) -> Self {
        let (tx, rx) = channel();

        thread::spawn(move || {
            let mut buffer = [0u8; 1];
            let stdin = io::stdin();
            let mut handle = stdin.lock();
            while handle.read_exact(&mut buffer).is_ok() {
                let _ = tx.send(buffer[0]);
            }
        });

        Self {
            base_addr,
            rx_queue: VecDeque::new(),
            rx_receiver: Mutex::new(rx),
            ier: 0,
            lcr: 0,
            mcr: 0,
            scr: 0,
            div: 0,
            tick_count: 0,
            thre_ip: true,
            tx_buffer: Vec::new(),
            to_stderr,
            panic_match_state: 0,
            panic_detected: false,
        }
    }

    /// Polls the stdin receiver and populates the RX queue.
    fn check_stdin(&mut self) {
        if let Ok(rx) = self.rx_receiver.lock() {
            while let Ok(byte) = rx.try_recv() {
                self.rx_queue.push_back(byte);
            }
        }
    }

    /// Calculates the Interrupt Identity Register (IIR) value.
    ///
    /// Determines the highest priority pending interrupt.
    fn update_interrupts(&mut self) -> u8 {
        if (self.ier & IER_RDA) != 0 && !self.rx_queue.is_empty() {
            return IIR_RDA;
        }

        if (self.ier & IER_THRE) != 0 && self.thre_ip {
            return IIR_THRE;
        }
        IIR_NO_INTERRUPT
    }

    /// Flushes the transmit buffer to stdout or stderr.
    fn flush_buffer(&mut self) {
        if !self.tx_buffer.is_empty() {
            let output: String = self.tx_buffer.iter().map(|&b| b as char).collect();
            if self.to_stderr {
                eprint!("{}", output);
                io::stderr().flush().ok();
            } else {
                print!("{}", output);
                io::stdout().flush().ok();
            }
            self.tx_buffer.clear();
        }
    }

    /// Scans output characters for the "kernel panic" string.
    ///
    /// Used to detect fatal errors in the guest OS and terminate simulation.
    fn check_char_for_panic(&mut self, ch: u8) -> bool {
        let ch_lower = if ch >= b'A' && ch <= b'Z' {
            ch + 32
        } else {
            ch
        };

        /// Pattern to detect kernel panic messages in UART output.
        const PATTERN: &[u8] = b"kernel panic";

        if ch_lower == PATTERN[self.panic_match_state] {
            self.panic_match_state += 1;
            if self.panic_match_state == PATTERN.len() {
                self.panic_detected = true;
                self.panic_match_state = 0;
                return true;
            }
        } else {
            if ch_lower == b'k' {
                self.panic_match_state = 1;
            } else {
                self.panic_match_state = 0;
            }
        }
        false
    }

    /// Returns true if a kernel panic has been detected in the output stream.
    pub fn check_kernel_panic(&mut self) -> bool {
        self.panic_detected
    }

    /// Checks if Divisor Latch Access Bit (DLAB) is set in LCR.
    fn dlab_set(&self) -> bool {
        (self.lcr & LCR_DLAB) != 0
    }

    /// Reads Receiver Buffer Register (RBR) or Divisor Latch Low (DLL).
    ///
    /// The register accessed depends on the DLAB bit in the LCR.
    fn read_rbr_or_dll(&mut self) -> u8 {
        if self.dlab_set() {
            (self.div & 0xFF) as u8
        } else {
            self.rx_queue.pop_front().unwrap_or(0)
        }
    }

    /// Reads Interrupt Enable Register (IER) or Divisor Latch High (DLM).
    ///
    /// The register accessed depends on the DLAB bit in the LCR.
    fn read_ier_or_dlm(&self) -> u8 {
        if self.dlab_set() {
            (self.div >> 8) as u8
        } else {
            self.ier
        }
    }

    /// Reads Interrupt Identity Register (IIR).
    ///
    /// Reading this register may clear pending interrupts (e.g., THRE).
    fn read_iir(&mut self) -> u8 {
        let iir = self.update_interrupts();
        if iir == IIR_THRE {
            self.thre_ip = false;
        }
        IIR_ID_MASK | iir
    }

    /// Reads Line Status Register (LSR).
    ///
    /// Indicates if data is ready or if the transmitter is empty.
    fn read_lsr(&self) -> u8 {
        let mut lsr = LSR_DEFAULT;
        if !self.rx_queue.is_empty() {
            lsr |= LSR_DATA_READY;
        }
        lsr
    }

    /// Writes Transmitter Holding Register (THR) or Divisor Latch Low (DLL).
    ///
    /// The register accessed depends on the DLAB bit in the LCR.
    fn write_thr_or_dll(&mut self, val: u8) {
        if self.dlab_set() {
            self.div = (self.div & 0xFF00) | (val as u16);
        } else {
            if self.check_char_for_panic(val) {
                self.flush_buffer();
                return;
            }

            self.tx_buffer.push(val);

            if val == b'\n' || self.tx_buffer.len() >= TX_BUFFER_FLUSH_THRESHOLD {
                self.flush_buffer();
            }

            self.thre_ip = true;
        }
    }

    /// Writes Interrupt Enable Register (IER) or Divisor Latch High (DLM).
    ///
    /// The register accessed depends on the DLAB bit in the LCR.
    fn write_ier_or_dlm(&mut self, val: u8) {
        if self.dlab_set() {
            self.div = (self.div & 0x00FF) | ((val as u16) << 8);
        } else {
            self.ier = val;
            if (self.ier & IER_THRE) != 0 {
                self.thre_ip = true;
            }
        }
    }
}

impl Drop for Uart {
    /// Flushes any remaining output when the UART is dropped.
    fn drop(&mut self) {
        self.flush_buffer();
    }
}

impl Device for Uart {
    /// Returns the device name.
    fn name(&self) -> &str {
        "UART0"
    }
    /// Returns the address range (Base, Size).
    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x100)
    }

    /// Reads a byte from the device.
    fn read_u8(&mut self, offset: u64) -> u8 {
        match offset {
            REG_RBR => self.read_rbr_or_dll(),
            REG_IER => self.read_ier_or_dlm(),
            REG_IIR => self.read_iir(),
            REG_LCR => self.lcr,
            REG_MCR => self.mcr,
            REG_LSR => self.read_lsr(),
            REG_MSR => 0,
            REG_SCR => self.scr,
            _ => 0,
        }
    }

    /// Reads a half-word (delegates to read_u8).
    fn read_u16(&mut self, offset: u64) -> u16 {
        self.read_u8(offset) as u16
    }
    /// Reads a word (delegates to read_u8).
    fn read_u32(&mut self, offset: u64) -> u32 {
        self.read_u8(offset) as u32
    }
    /// Reads a double-word (delegates to read_u8).
    fn read_u64(&mut self, offset: u64) -> u64 {
        self.read_u8(offset) as u64
    }

    /// Writes a byte to the device.
    fn write_u8(&mut self, offset: u64, val: u8) {
        match offset {
            REG_THR => self.write_thr_or_dll(val),
            REG_IER => self.write_ier_or_dlm(val),
            REG_FCR => {}
            REG_LCR => self.lcr = val,
            REG_MCR => self.mcr = val,
            REG_SCR => self.scr = val,
            _ => {}
        }
    }

    /// Writes a half-word (delegates to write_u8).
    fn write_u16(&mut self, offset: u64, val: u16) {
        self.write_u8(offset, val as u8);
    }
    /// Writes a word (delegates to write_u8).
    fn write_u32(&mut self, offset: u64, val: u32) {
        self.write_u8(offset, val as u8);
    }
    /// Writes a double-word (delegates to write_u8).
    fn write_u64(&mut self, offset: u64, val: u64) {
        self.write_u8(offset, val as u8);
    }

    /// Advances the device state.
    ///
    /// Polls stdin periodically and returns true if an interrupt is pending.
    fn tick(&mut self) -> bool {
        self.tick_count = self.tick_count.wrapping_add(1);
        if self.tick_count == 0 {
            self.check_stdin();
        }

        let iir = self.update_interrupts();
        (iir & IIR_NO_INTERRUPT) == 0
    }

    /// Returns the Interrupt Request (IRQ) ID associated with this device.
    fn get_irq_id(&self) -> Option<u32> {
        Some(10)
    }

    /// Returns a mutable reference to the UART if this device is one.
    fn as_uart_mut(&mut self) -> Option<&mut Uart> {
        Some(self)
    }
}
