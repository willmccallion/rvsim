//! Host-Target Interface (HTIF) device.
//!
//! Implements the HTIF tohost/fromhost protocol used by riscv-tests and other
//! bare-metal test suites. The test program writes a result value to the
//! `tohost` memory-mapped address:
//!
//! * `1` — test passed (exit code 0).
//! * Odd and not 1 — test failed; the failing test number is `value >> 1`.
//! * `0` — ignored (tests poll-write zero before writing the real value).
//!
//! This device occupies a single 8-byte slot on the bus at the address of the
//! `tohost` ELF symbol. It shares the same `exit_request` atomic as SysCon so
//! the simulation loop picks up the exit without any extra plumbing.

use crate::soc::devices::Device;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// HTIF device: intercepts writes to the `tohost` address.
pub struct Htif {
    base_addr: u64,
    exit_signal: Arc<AtomicU64>,
}

impl Htif {
    /// Creates a new HTIF device at `base_addr` using the shared exit signal.
    pub fn new(base_addr: u64, exit_signal: Arc<AtomicU64>) -> Self {
        Self {
            base_addr,
            exit_signal,
        }
    }

    fn handle_tohost(&self, val: u64) {
        if val == 0 {
            return;
        }
        if val == 1 {
            // Pass
            self.exit_signal.store(0, Ordering::Relaxed);
        } else if val & 1 != 0 {
            // Fail — test number is val >> 1
            let test_num = val >> 1;
            eprintln!("[HTIF] FAIL: test case {} (tohost={:#x})", test_num, val);
            self.exit_signal.store(test_num, Ordering::Relaxed);
        } else {
            // Even non-zero values are device commands in full HTIF.
            // For riscv-tests we only care about the above cases, but store
            // the raw value so the simulation exits rather than spinning.
            eprintln!("[HTIF] Unhandled tohost value: {:#x}", val);
            self.exit_signal.store(val, Ordering::Relaxed);
        }
    }
}

impl Device for Htif {
    fn name(&self) -> &str {
        "HTIF"
    }

    fn address_range(&self) -> (u64, u64) {
        // tohost (8 bytes) + fromhost (8 bytes)
        (self.base_addr, 16)
    }

    fn read_u8(&mut self, _offset: u64) -> u8 {
        0
    }
    fn read_u16(&mut self, _offset: u64) -> u16 {
        0
    }
    fn read_u32(&mut self, _offset: u64) -> u32 {
        0
    }
    fn read_u64(&mut self, _offset: u64) -> u64 {
        0
    }

    fn write_u8(&mut self, _offset: u64, _val: u8) {}
    fn write_u16(&mut self, _offset: u64, _val: u16) {}

    fn write_u32(&mut self, offset: u64, val: u32) {
        if offset == 0 {
            self.handle_tohost(val as u64);
        }
    }

    fn write_u64(&mut self, offset: u64, val: u64) {
        if offset == 0 {
            self.handle_tohost(val);
        }
    }
}
