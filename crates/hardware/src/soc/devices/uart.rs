//! Universal Asynchronous Receiver-Transmitter (UART).
//!
//! Implements a 16550-compatible UART device for serial communication.
//! Handles standard registers (RBR, THR, IER, IIR, LCR, LSR) and integrates
//! with stdin/stdout for console I/O. Output leaves at once; the transmit
//! and receive interrupts rise 225 ns after their cause, as in gem5.

use serde::{Deserialize, Serialize};

use crate::common::{IrqId, LineAddr};
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData};
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
/// FIFO Control Register (Write) — acknowledged but not implemented.
const _REG_FCR: u64 = 2;
/// Line Control Register.
const REG_LCR: u64 = 3;
/// Modem Control Register.
const REG_MCR: u64 = 4;
/// Line Status Register.
const REG_LSR: u64 = 5;
/// Modem Status Register — reads return 0.
const _REG_MSR: u64 = 6;
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

/// Time from an interrupt's cause to the line rising (gem5's `Uart8250`
/// schedules both its interrupts this far ahead).
const INTERRUPT_DELAY_NS: u64 = 225;

/// Interrupt Enable Register: Transmitter Holding Register Empty interrupt enable.
const IER_THRE: u8 = 0x02;

/// UART device structure.
///
/// Simulates a 16550 UART. It spawns a background thread to capture `stdin`
/// for input and writes output directly to `stdout`.
#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
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
    /// Cycles since reset.
    cycle: u64,
    /// Cycles between an interrupt's cause and the line rising.
    interrupt_delay: u64,
    /// The cycle the transmit-empty interrupt rises, once scheduled.
    tx_interrupt_at: Option<u64>,
    /// The cycle the receive-data interrupt rises, once scheduled.
    rx_interrupt_at: Option<u64>,
    /// Transmitter Holding Register Empty Interrupt Pending.
    thre_ip: bool,
    /// Received data has been announced by the receive interrupt.
    rx_ready: bool,
    /// When true, output goes to stderr (for visibility when run from Python).
    to_stderr: bool,
    /// When true, all output is suppressed (for scripting / benchmarks).
    quiet: bool,
    /// State machine index for panic detection.
    panic_match_state: usize,
    /// Flag indicating if a kernel panic string was detected.
    panic_detected: bool,
}

/// The UART's registers, interrupt timing and received data, as a
/// checkpoint carries them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UartState {
    /// Interrupt Enable Register.
    pub ier: u8,
    /// Line Control Register.
    pub lcr: u8,
    /// Modem Control Register.
    pub mcr: u8,
    /// Scratch Register.
    pub scr: u8,
    /// Divisor latch.
    pub div: u16,
    /// Cycles since reset.
    pub cycle: u64,
    /// The cycle the transmit-empty interrupt rises, if scheduled.
    pub tx_interrupt_at: Option<u64>,
    /// The cycle the receive-data interrupt rises, if scheduled.
    pub rx_interrupt_at: Option<u64>,
    /// Transmit-empty interrupt pending.
    pub thre_ip: bool,
    /// Received data announced.
    pub rx_ready: bool,
    /// Received bytes not yet read.
    pub rx_queue: Vec<u8>,
}

impl Uart {
    /// The registers, interrupt timing and received data a checkpoint carries.
    #[must_use]
    pub fn state(&self) -> UartState {
        UartState {
            ier: self.ier,
            lcr: self.lcr,
            mcr: self.mcr,
            scr: self.scr,
            div: self.div,
            cycle: self.cycle,
            tx_interrupt_at: self.tx_interrupt_at,
            rx_interrupt_at: self.rx_interrupt_at,
            thre_ip: self.thre_ip,
            rx_ready: self.rx_ready,
            rx_queue: self.rx_queue.iter().copied().collect(),
        }
    }

    /// Restores registers, interrupt timing and received data from a checkpoint.
    pub fn set_state(&mut self, state: &UartState) {
        self.ier = state.ier;
        self.lcr = state.lcr;
        self.mcr = state.mcr;
        self.scr = state.scr;
        self.div = state.div;
        self.cycle = state.cycle;
        self.tx_interrupt_at = state.tx_interrupt_at;
        self.rx_interrupt_at = state.rx_interrupt_at;
        self.thre_ip = state.thre_ip;
        self.rx_ready = state.rx_ready;
        self.rx_queue = state.rx_queue.iter().copied().collect();
    }

    /// Creates a new UART device, spawning a background thread to read stdin.
    /// `cpu_clock_mhz` sizes the interrupt delay in cycles.
    pub fn new(base_addr: u64, to_stderr: bool, quiet: bool, cpu_clock_mhz: u64) -> Self {
        let (tx, rx) = channel();

        let _ = thread::spawn(move || {
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
            cycle: 0,
            interrupt_delay: INTERRUPT_DELAY_NS * cpu_clock_mhz / 1000,
            tx_interrupt_at: None,
            rx_interrupt_at: None,
            thre_ip: true,
            rx_ready: false,
            to_stderr,
            quiet,
            panic_match_state: 0,
            panic_detected: false,
        }
    }

    /// Polls the stdin receiver and populates the RX queue; newly arrived
    /// data raises the receive interrupt after the delay.
    fn check_stdin(&mut self) {
        let Ok(rx) = self.rx_receiver.lock() else { return };
        let mut arrived = false;
        while let Ok(byte) = rx.try_recv() {
            self.rx_queue.push_back(byte);
            arrived = true;
        }
        if arrived && !self.rx_ready && self.rx_interrupt_at.is_none() {
            self.rx_interrupt_at = Some(self.cycle + self.interrupt_delay);
        }
    }

    /// Schedules the transmit-empty interrupt for after the delay.
    const fn schedule_tx_interrupt(&mut self) {
        self.thre_ip = false;
        self.tx_interrupt_at = Some(self.cycle + self.interrupt_delay);
    }

    /// Raises the interrupts whose delay has elapsed.
    fn raise_due_interrupts(&mut self) {
        if self.tx_interrupt_at.is_some_and(|at| at <= self.cycle) {
            self.tx_interrupt_at = None;
            self.thre_ip = true;
        }
        if self.rx_interrupt_at.is_some_and(|at| at <= self.cycle) {
            self.rx_interrupt_at = None;
            self.rx_ready = true;
        }
    }

    /// Calculates the Interrupt Identity Register (IIR) value (highest priority pending interrupt).
    fn update_interrupts(&self) -> u8 {
        if (self.ier & IER_RDA) != 0 && self.rx_ready && !self.rx_queue.is_empty() {
            return IIR_RDA;
        }

        if (self.ier & IER_THRE) != 0 && self.thre_ip {
            return IIR_THRE;
        }
        IIR_NO_INTERRUPT
    }

    /// Scans output characters for the "kernel panic" string.
    ///
    /// Used to detect fatal errors in the guest OS and terminate simulation.
    fn check_char_for_panic(&mut self, ch: u8) -> bool {
        /// Pattern to detect kernel panic messages in UART output.
        const PATTERN: &[u8] = b"kernel panic";
        let ch_lower = if ch.is_ascii_uppercase() { ch + 32 } else { ch };

        if ch_lower == PATTERN[self.panic_match_state] {
            self.panic_match_state += 1;
            if self.panic_match_state == PATTERN.len() {
                self.panic_detected = true;
                self.panic_match_state = 0;
                return true;
            }
        } else if ch_lower == b'k' {
            self.panic_match_state = 1;
        } else {
            self.panic_match_state = 0;
        }
        false
    }

    /// Returns true if a kernel panic has been detected in the output stream.
    pub const fn check_kernel_panic(&mut self) -> bool {
        self.panic_detected
    }

    /// Checks if Divisor Latch Access Bit (DLAB) is set in LCR.
    const fn dlab_set(&self) -> bool {
        (self.lcr & LCR_DLAB) != 0
    }

    /// Reads Receiver Buffer Register (RBR) or Divisor Latch Low (DLL) based on DLAB.
    fn read_rbr_or_dll(&mut self) -> u8 {
        if self.dlab_set() {
            return (self.div & 0xFF) as u8;
        }
        let byte = self.rx_queue.pop_front().unwrap_or(0);
        if self.rx_queue.is_empty() {
            self.rx_ready = false;
        }
        byte
    }

    /// Reads Interrupt Enable Register (IER) or Divisor Latch High (DLM) based on DLAB.
    const fn read_ier_or_dlm(&self) -> u8 {
        if self.dlab_set() { (self.div >> 8) as u8 } else { self.ier }
    }

    /// Reads Interrupt Identity Register (IIR), clearing THRE if pending.
    fn read_iir(&mut self) -> u8 {
        let iir = self.update_interrupts();
        if iir == IIR_THRE {
            self.thre_ip = false;
        }
        IIR_ID_MASK | iir
    }

    /// Reads Line Status Register (LSR).
    fn read_lsr(&self) -> u8 {
        let mut lsr = LSR_DEFAULT;
        if !self.rx_queue.is_empty() {
            lsr |= LSR_DATA_READY;
        }
        lsr
    }

    /// Writes Transmitter Holding Register (THR) or Divisor Latch Low (DLL) based on DLAB.
    fn write_thr_or_dll(&mut self, val: u8) {
        if self.dlab_set() {
            self.div = (self.div & 0xFF00) | (val as u16);
        } else {
            if self.check_char_for_panic(val) {
                return;
            }

            if !self.quiet {
                if self.to_stderr {
                    eprint!("{}", val as char);
                    let _ = io::stderr().flush();
                } else {
                    print!("{}", val as char);
                    let _ = io::stdout().flush();
                }
            }

            self.schedule_tx_interrupt();
        }
    }

    /// Writes Interrupt Enable Register (IER) or Divisor Latch High (DLM) based on DLAB.
    const fn write_ier_or_dlm(&mut self, val: u8) {
        if self.dlab_set() {
            self.div = (self.div & 0x00FF) | ((val as u16) << 8);
        } else {
            self.ier = val;
            if (self.ier & IER_THRE) != 0 {
                self.schedule_tx_interrupt();
            }
        }
    }
}

impl Uart {
    fn read_register(&mut self, offset: u64) -> u8 {
        match offset {
            REG_RBR => self.read_rbr_or_dll(),
            REG_IER => self.read_ier_or_dlm(),
            REG_IIR => self.read_iir(),
            REG_LCR => self.lcr,
            REG_MCR => self.mcr,
            REG_LSR => self.read_lsr(),
            REG_SCR => self.scr,
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, val: u8) {
        match offset {
            REG_THR => self.write_thr_or_dll(val),
            REG_IER => self.write_ier_or_dlm(val),
            REG_LCR => self.lcr = val,
            REG_MCR => self.mcr = val,
            REG_SCR => self.scr = val,
            _ => {}
        }
    }
}

impl Handle for Uart {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base_addr);
            let value: u64 = match op {
                MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. } => {
                    u64::from(self.read_register(offset))
                }
                MemOp::Write { data: WriteData::Small(val) } => {
                    self.write_register(offset, val as u8);
                    0
                }
                MemOp::Write { .. } | MemOp::Writeback { .. } => 0,
            };
            ctx.scheduler.schedule(
                ctx.cycle + 1,
                source,
                ctx.self_id,
                Packet::MemResp {
                    req_id,
                    line_addr: LineAddr::from_phys(paddr, 64),
                    data: MemRespData::Small(value),
                    hit_level: HitLevel::Mmio,
                    state: MesiState::Exclusive,
                },
            );
        }
    }
}

impl Device for Uart {
    fn name(&self) -> &'static str {
        "UART0"
    }
    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x100)
    }

    fn tick(&mut self) -> bool {
        self.cycle += 1;
        self.tick_count = self.tick_count.wrapping_add(1);
        if self.tick_count == 0 {
            self.check_stdin();
        }
        self.raise_due_interrupts();

        let iir = self.update_interrupts();
        (iir & IIR_NO_INTERRUPT) == 0
    }

    fn get_irq_id(&self) -> Option<IrqId> {
        Some(IrqId::new(10))
    }

    fn checkpoint(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self.state()).ok()
    }

    fn restore(&mut self, state: &serde_json::Value) {
        if let Ok(state) = serde_json::from_value::<UartState>(state.clone()) {
            self.set_state(&state);
        }
    }

    fn as_uart_mut(&mut self) -> Option<&mut Uart> {
        Some(self)
    }
}
