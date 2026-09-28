//! System Controller (SysCon).
//!
//! A simple memory-mapped device used to control system power and reset states.
//! It is primarily used by the kernel or test environment to gracefully exit
//! the simulation or trigger a reset.
//!
//! # Registers
//!
//! * `0x00`: Command Register (Write Only)
//!   * `0x5555`: Power Off
//!   * `0x7777`: Reset
//!   * `0x3333`: Failure/Panic

use crate::common::LineAddr;
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData};
use crate::soc::devices::Device;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// `SysCon` device structure.
#[derive(Debug)]
pub struct SysCon {
    /// Base physical address of the device.
    base_addr: u64,
    /// Shared atomic flag to signal the simulation loop to exit.
    exit_signal: Arc<AtomicU64>,
}

impl SysCon {
    /// Creates a new `SysCon` device.
    pub const fn new(base_addr: u64, exit_signal: Arc<AtomicU64>) -> Self {
        Self { base_addr, exit_signal }
    }

    fn act_on_command(&self, val: u32) {
        match val {
            0x5555 => {
                println!("[SysCon] Poweroff signal received.");
                self.exit_signal.store(0, Ordering::Relaxed);
            }
            0x7777 => {
                println!("[SysCon] Reset signal received (Simulated as Exit).");
                self.exit_signal.store(0, Ordering::Relaxed);
            }
            0x3333 => {
                println!("[SysCon] Failure signal received.");
                self.exit_signal.store(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

impl Handle for SysCon {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base_addr);
            // SysCon commands are 32-bit (0x5555 / 0x7777 / 0x3333). Sub-word
            // partial writes that don't reach a full register width are
            // discarded, mirroring real device behaviour and what tests rely on.
            if offset == 0
                && matches!(
                    size,
                    crate::sim::packet::AccessSize::B4 | crate::sim::packet::AccessSize::B8
                )
                && let MemOp::Write { data: WriteData::Small(val) } = op
            {
                self.act_on_command(val as u32);
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

impl Device for SysCon {
    fn name(&self) -> &'static str {
        "SysCon"
    }

    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x1000)
    }
}
