//! The framing of GDB's remote serial protocol: `$payload#checksum`
//! packets, their acknowledgements, and the interrupt byte GDB sends
//! while the target runs.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// GDB's interrupt: Ctrl-C, sent as a bare byte while the target runs.
const INTERRUPT: u8 = 0x03;
const ACK: u8 = b'+';
const NAK: u8 = b'-';
/// How long a wait for the next packet lasts before the host is asked
/// whether to go on waiting.
const IDLE_POLL: Duration = Duration::from_millis(200);

/// What arrived from the debugger.
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    /// A packet's payload, checksum verified.
    Packet(Vec<u8>),
    /// The interrupt byte.
    Interrupt,
    /// Nothing, for a while.
    Idle,
    /// The debugger closed the connection.
    Closed,
}

enum Byte {
    Read(u8),
    Closed,
    TimedOut,
}

/// The connection to the debugger, framing packets both ways.
#[derive(Debug)]
pub struct Transport {
    stream: TcpStream,
    /// Whether packets are acknowledged; `QStartNoAckMode` turns it off.
    acks: bool,
    /// Bytes read ahead of the current packet.
    pending: Vec<u8>,
}

impl Transport {
    /// Frames packets over `stream`.
    pub const fn new(stream: TcpStream) -> Self {
        Self { stream, acks: true, pending: Vec::new() }
    }

    /// Stops acknowledging packets, as `QStartNoAckMode` asks.
    pub const fn stop_acknowledging(&mut self) {
        self.acks = false;
    }

    /// Waits for the next packet, interrupt or close, or until the idle
    /// poll passes with nothing.
    ///
    /// # Errors
    ///
    /// The connection's I/O error.
    pub fn receive(&mut self) -> io::Result<Incoming> {
        self.stream.set_nonblocking(false)?;
        self.stream.set_read_timeout(Some(IDLE_POLL))?;
        loop {
            let byte = match self.next_byte()? {
                Byte::Read(byte) => byte,
                Byte::Closed => return Ok(Incoming::Closed),
                Byte::TimedOut => return Ok(Incoming::Idle),
            };
            match byte {
                INTERRUPT => return Ok(Incoming::Interrupt),
                b'$' => {
                    if let Some(payload) = self.read_payload()? {
                        return Ok(Incoming::Packet(payload));
                    }
                }
                // Acknowledgements of our own packets, and noise.
                _ => {}
            }
        }
    }

    /// Reads a packet body after its `$`, checks the checksum, and
    /// acknowledges it; `None` when the checksum was wrong and a resend
    /// was asked for.
    fn read_payload(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut payload = Vec::new();
        loop {
            let Some(byte) = self.next_byte_blocking()? else {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "packet cut short"));
            };
            if byte == b'#' {
                break;
            }
            payload.push(byte);
        }
        let mut sum = [0u8; 2];
        for slot in &mut sum {
            *slot = self.next_byte_blocking()?.ok_or_else(|| {
                io::Error::new(io::ErrorKind::UnexpectedEof, "checksum cut short")
            })?;
        }
        let expected = checksum(&payload);
        let given = u8::from_str_radix(std::str::from_utf8(&sum).unwrap_or("zz"), 16).ok();
        if given != Some(expected) {
            if self.acks {
                self.stream.write_all(&[NAK])?;
            }
            return Ok(None);
        }
        if self.acks {
            self.stream.write_all(&[ACK])?;
        }
        Ok(Some(unescape(&payload)))
    }

    /// Sends `payload` as a packet, resending on a `-` until acknowledged
    /// while acknowledgements are on.
    ///
    /// # Errors
    ///
    /// The connection's I/O error.
    pub fn send(&mut self, payload: &[u8]) -> io::Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.push(b'$');
        frame.extend_from_slice(payload);
        frame.push(b'#');
        frame.extend_from_slice(format!("{:02x}", checksum(payload)).as_bytes());
        self.stream.set_nonblocking(false)?;
        loop {
            self.stream.write_all(&frame)?;
            if !self.acks {
                return Ok(());
            }
            loop {
                match self.next_byte_blocking()? {
                    Some(ACK) | None => return Ok(()),
                    Some(NAK) => break,
                    Some(INTERRUPT) => self.pending.push(INTERRUPT),
                    Some(_) => {}
                }
            }
        }
    }

    /// Whether the debugger has sent the interrupt byte; anything else
    /// that arrived is kept for the next packet. Does not wait.
    pub fn interrupted(&mut self) -> bool {
        if let Some(at) = self.pending.iter().position(|&byte| byte == INTERRUPT) {
            let _ = self.pending.remove(at);
            return true;
        }
        if self.stream.set_nonblocking(true).is_err() {
            return false;
        }
        let mut buffer = [0u8; 64];
        let interrupted = match self.stream.read(&mut buffer) {
            Ok(n) if n > 0 => {
                let bytes = &buffer[..n];
                self.pending.extend(bytes.iter().filter(|&&byte| byte != INTERRUPT));
                bytes.contains(&INTERRUPT)
            }
            Ok(_) | Err(_) => false,
        };
        let _ = self.stream.set_nonblocking(false);
        interrupted
    }

    /// The next byte, waiting past read timeouts; `None` once the
    /// debugger has closed the connection.
    fn next_byte_blocking(&mut self) -> io::Result<Option<u8>> {
        loop {
            match self.next_byte()? {
                Byte::Read(byte) => return Ok(Some(byte)),
                Byte::Closed => return Ok(None),
                Byte::TimedOut => {}
            }
        }
    }

    fn next_byte(&mut self) -> io::Result<Byte> {
        if !self.pending.is_empty() {
            return Ok(Byte::Read(self.pending.remove(0)));
        }
        let mut byte = [0u8; 1];
        match self.stream.read(&mut byte) {
            Ok(0) => Ok(Byte::Closed),
            Ok(_) => Ok(Byte::Read(byte[0])),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                Ok(Byte::TimedOut)
            }
            Err(e) => Err(e),
        }
    }
}

/// The protocol's checksum: the payload's bytes summed modulo 256.
fn checksum(payload: &[u8]) -> u8 {
    payload.iter().fold(0u8, |sum, &byte| sum.wrapping_add(byte))
}

/// Undoes the protocol's `}` escaping, which GDB uses for binary data.
fn unescape(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len());
    let mut bytes = payload.iter();
    while let Some(&byte) = bytes.next() {
        if byte == b'}' {
            if let Some(&next) = bytes.next() {
                out.push(next ^ 0x20);
            }
        } else {
            out.push(byte);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksum_is_the_byte_sum_modulo_256() {
        assert_eq!(checksum(b"g"), 0x67);
        assert_eq!(
            checksum(b"qSupported"),
            b"qSupported".iter().map(|&b| u32::from(b)).sum::<u32>() as u8
        );
    }

    #[test]
    fn escaped_bytes_are_restored() {
        assert_eq!(unescape(b"a}\x5db}\x03c"), b"a}b#c");
    }
}
