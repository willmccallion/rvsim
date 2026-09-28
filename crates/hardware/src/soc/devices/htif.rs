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
//! This device occupies a single 16-byte slot on the bus at the address of the
//! `tohost` ELF symbol. It shares the same `exit_request` atomic as SysCon so
//! the simulation loop picks up the exit without any extra plumbing.

use crate::common::LineAddr;
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData};
use crate::soc::devices::Device;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// HTIF device: intercepts writes to the `tohost` address.
#[derive(Debug)]
pub struct Htif {
    base_addr: u64,
    exit_signal: Arc<AtomicU64>,
}

impl Htif {
    /// Creates a new HTIF device at `base_addr` using the shared exit signal.
    pub const fn new(base_addr: u64, exit_signal: Arc<AtomicU64>) -> Self {
        Self { base_addr, exit_signal }
    }

    fn handle_tohost(&self, val: u64) {
        if val == 0 {
            return;
        }
        if val == 1 {
            self.exit_signal.store(0, Ordering::Relaxed);
        } else if val & 1 != 0 {
            let test_num = val >> 1;
            eprintln!("[HTIF] FAIL: test case {test_num} (tohost={val:#x})");
            self.exit_signal.store(test_num, Ordering::Relaxed);
        } else {
            eprintln!("[HTIF] Unhandled tohost value: {val:#x}");
            self.exit_signal.store(val, Ordering::Relaxed);
        }
    }
}

impl Handle for Htif {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base_addr);
            // Only word- or doubleword-aligned writes to the tohost register
            // (offset 0) trigger the protocol; spike's HTIF ignores sub-word
            // partial writes and all writes to non-zero offsets in the slot.
            if offset == 0
                && matches!(
                    size,
                    crate::sim::packet::AccessSize::B4 | crate::sim::packet::AccessSize::B8
                )
                && let MemOp::Write { data: WriteData::Small(val), .. } = op
            {
                self.handle_tohost(val);
            }
            ctx.scheduler.schedule(
                ctx.cycle + ctx.config.system.device_access_cycles(self.name()),
                source,
                ctx.self_id,
                Packet::MemResp {
                    req_id,
                    line_addr: LineAddr::from_phys(paddr, 64),
                    data: MemRespData::Small(0),
                    hit_level: HitLevel::Mmio,
                    state: MesiState::Exclusive,
                },
            );
        }
    }
}

impl Device for Htif {
    fn name(&self) -> &'static str {
        "HTIF"
    }

    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 16)
    }
}
