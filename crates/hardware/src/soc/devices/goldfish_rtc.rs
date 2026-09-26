//! Goldfish Real-Time Clock (RTC).
//!
//! A virtual RTC device commonly used in Android emulators (QEMU).
//! It provides the current system time in nanoseconds.
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
use std::time::{SystemTime, UNIX_EPOCH};

/// Goldfish RTC device structure.
#[derive(Debug)]
pub struct GoldfishRtc {
    /// Base physical address of the device.
    base_addr: u64,
}

impl GoldfishRtc {
    /// Creates a new Goldfish RTC device.
    pub const fn new(base_addr: u64) -> Self {
        Self { base_addr }
    }

    /// Retrieves the current system time in nanoseconds.
    fn get_time_ns() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64
    }
}

impl Handle for GoldfishRtc {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base_addr);
            let value: u64 = match op {
                MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. } => {
                    match (offset, size) {
                        (0x00, AccessSize::B4) => u64::from(Self::get_time_ns() as u32),
                        (0x04, AccessSize::B4) => u64::from((Self::get_time_ns() >> 32) as u32),
                        (0x00, AccessSize::B8) => Self::get_time_ns(),
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
