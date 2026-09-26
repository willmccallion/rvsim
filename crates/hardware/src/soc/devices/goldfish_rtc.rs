//! Goldfish Real-Time Clock (RTC).
//!
//! A virtual RTC device commonly used in Android emulators (QEMU).
//! It reports wall-clock time in nanoseconds: a configured epoch plus the
//! simulated time elapsed, so a run reads the same clock every time.
//!
//! # Memory Map
//!
//! * `0x00`: Time (Low 32 bits)
//! * `0x04`: Time (High 32 bits)

use crate::common::{IrqId, LineAddr};
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, HitLevel, MemOp, MemRespData, MesiState, Packet};
use crate::soc::devices::Device;

/// Goldfish RTC device structure.
#[derive(Debug)]
pub struct GoldfishRtc {
    /// Base physical address of the device.
    base_addr: u64,
    /// Wall-clock time at cycle zero, in nanoseconds since the Unix epoch.
    epoch_ns: u64,
    /// Core clock, to convert cycles to nanoseconds.
    cpu_clock_mhz: u64,
}

impl GoldfishRtc {
    /// Creates a new Goldfish RTC device reading `epoch_ns` at cycle zero.
    pub const fn new(base_addr: u64, epoch_ns: u64, cpu_clock_mhz: u64) -> Self {
        Self {
            base_addr,
            epoch_ns,
            cpu_clock_mhz: if cpu_clock_mhz == 0 { 1 } else { cpu_clock_mhz },
        }
    }

    /// Wall-clock time at `cycle`, in nanoseconds.
    pub const fn time_ns(&self, cycle: u64) -> u64 {
        let elapsed = (cycle as u128 * 1000) / self.cpu_clock_mhz as u128;
        self.epoch_ns.wrapping_add(elapsed as u64)
    }
}

impl Handle for GoldfishRtc {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base_addr);
            let value: u64 = match op {
                MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. } => {
                    let now = self.time_ns(ctx.cycle);
                    match (offset, size) {
                        (0x00, AccessSize::B4) => u64::from(now as u32),
                        (0x04, AccessSize::B4) => u64::from((now >> 32) as u32),
                        (0x00, AccessSize::B8) => now,
                        _ => 0,
                    }
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

impl Device for GoldfishRtc {
    fn name(&self) -> &'static str {
        "GoldfishRTC"
    }

    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x1000)
    }

    fn get_irq_id(&self) -> Option<IrqId> {
        Some(IrqId::new(11))
    }
}
