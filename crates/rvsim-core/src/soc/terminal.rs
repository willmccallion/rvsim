//! The host-side terminal every path to the guest's console shares.
//!
//! The UART, the HTIF proxy and a user process's console descriptors all
//! write here and take typed input from here, so a quiet or captured
//! console, and the host's view of it, mean the same whichever way a
//! program prints. Input goes to one reader at a time: the UART, as on a
//! board, until a program that talks to the host directly (over HTIF, or
//! as a user process) is loaded, whose `read` of the console then gets it.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::config::Console;

/// Who reads the console's input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputReader {
    /// The UART, polled by the guest's driver.
    Uart,
    /// The syscall proxy: HTIF's `read`, or a user process's.
    Proxy,
}

/// One terminal, shared by every device and proxy that reaches it.
pub type SharedTerminal = Arc<Mutex<Terminal>>;

/// Where the guest's console output goes and its input comes from.
#[derive(Debug)]
pub struct Terminal {
    mode: Console,
    reader: InputReader,
    /// Output a captured console has written that the host has not taken.
    output: Vec<u8>,
    /// Input the host typed into the console, not yet read.
    typed: VecDeque<u8>,
    /// Bytes from the host's stdin, read by a thread so polling never blocks.
    stdin: Option<Receiver<u8>>,
}

impl Terminal {
    /// A terminal in `mode`, reading the host's stdin when the mode uses it.
    #[must_use]
    pub fn new(mode: Console) -> Self {
        let stdin = matches!(mode, Console::Stdout | Console::Stderr).then(spawn_stdin_reader);
        Self { mode, reader: InputReader::Uart, output: Vec::new(), typed: VecDeque::new(), stdin }
    }

    /// A new terminal in `mode`, ready to share.
    #[must_use]
    pub fn shared(mode: Console) -> SharedTerminal {
        Arc::new(Mutex::new(Self::new(mode)))
    }

    #[must_use]
    pub const fn mode(&self) -> Console {
        self.mode
    }

    /// Hands the console's input to `reader`.
    pub const fn read_input_through(&mut self, reader: InputReader) {
        self.reader = reader;
    }

    /// Writes `bytes` where the console goes.
    ///
    /// # Errors
    ///
    /// The host's stdout or stderr could not be written.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self.mode {
            Console::Stdout => {
                let mut out = io::stdout().lock();
                out.write_all(bytes)?;
                out.flush()
            }
            Console::Stderr => {
                let mut err = io::stderr().lock();
                err.write_all(bytes)?;
                err.flush()
            }
            Console::Quiet => Ok(()),
            Console::Captured => {
                self.output.extend_from_slice(bytes);
                Ok(())
            }
        }
    }

    /// One byte of input for the UART, while it is the reader.
    pub fn poll_input(&mut self) -> Option<u8> {
        if self.reader != InputReader::Uart {
            return None;
        }
        self.next_byte()
    }

    /// Whether the UART, as the reader, may have input to poll for.
    #[must_use]
    pub fn uart_may_receive(&self) -> bool {
        self.reader == InputReader::Uart && (self.stdin.is_some() || !self.typed.is_empty())
    }

    /// Up to `max` bytes for a proxy's `read` of the console. Waits for
    /// the first byte from the host's stdin, as the read would on Linux;
    /// a captured console gives what the host has typed, and nothing,
    /// the end of input, when it has typed nothing.
    pub fn read_input(&mut self, max: usize) -> Vec<u8> {
        if self.reader != InputReader::Proxy || max == 0 {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        if self.typed.is_empty()
            && let Some(stdin) = &self.stdin
            && let Ok(byte) = stdin.recv()
        {
            bytes.push(byte);
        }
        while bytes.len() < max {
            match self.next_byte() {
                Some(byte) => bytes.push(byte),
                None => break,
            }
        }
        bytes
    }

    /// Types `bytes` into the console, as the host of a captured console does.
    pub fn type_input(&mut self, bytes: &[u8]) {
        self.typed.extend(bytes);
    }

    /// Whether a captured console holds output the host has not taken.
    #[must_use]
    pub const fn has_output(&self) -> bool {
        !self.output.is_empty()
    }

    /// Takes the output a captured console has written since the last call.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    fn next_byte(&mut self) -> Option<u8> {
        if let Some(byte) = self.typed.pop_front() {
            return Some(byte);
        }
        self.stdin.as_ref().and_then(|stdin| stdin.try_recv().ok())
    }
}

/// Reads the host's stdin a byte at a time on its own thread, so the
/// simulator can poll for input without blocking.
fn spawn_stdin_reader() -> Receiver<u8> {
    let (tx, rx) = channel();
    let _ = thread::spawn(move || {
        let mut buffer = [0u8; 1];
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        while handle.read_exact(&mut buffer).is_ok() {
            if tx.send(buffer[0]).is_err() {
                break;
            }
        }
    });
    rx
}
