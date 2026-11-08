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

use std::num::NonZeroU64;

use crate::common::{IrqId, PhysAddr, PhysRange};
use crate::config::DeviceKind;
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, MemOp, Packet};
use crate::soc::devices::Device;
use crate::soc::traits::{MmioRegisters, serve_mmio};

/// Goldfish RTC device structure.
#[derive(Debug)]
pub struct GoldfishRtc {
    /// Base physical address of the device.
    base_addr: PhysAddr,
    /// Wall-clock time at cycle zero, in nanoseconds since the Unix epoch.
    epoch_ns: u64,
    /// Core clock, to convert cycles to nanoseconds.
    cpu_clock_mhz: NonZeroU64,
}

impl GoldfishRtc {
    /// The PLIC source the RTC raises.
    pub const IRQ: IrqId = IrqId::new(11);

    /// Creates a new Goldfish RTC device reading `epoch_ns` at cycle zero.
    pub const fn new(base_addr: PhysAddr, epoch_ns: u64, cpu_clock_mhz: NonZeroU64) -> Self {
        Self { base_addr, epoch_ns, cpu_clock_mhz }
    }

    /// Wall-clock time at `cycle`, in nanoseconds.
    pub const fn time_ns(&self, cycle: u64) -> u64 {
        let elapsed = (cycle as u128 * 1000) / self.cpu_clock_mhz.get() as u128;
        self.epoch_ns.wrapping_add(elapsed as u64)
    }
}

impl Handle for GoldfishRtc {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        serve_mmio(self, packet, source, ctx);
    }
}

impl MmioRegisters for GoldfishRtc {
    fn access(&mut self, offset: u64, size: AccessSize, op: MemOp, ctx: &mut HandleCtx<'_>) -> u64 {
        match op {
            MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. } => {
                let now = self.time_ns(ctx.cycle);
                match (offset, size) {
                    (0x00, AccessSize::B4) => u64::from(now as u32),
                    (0x04, AccessSize::B4) => u64::from((now >> 32) as u32),
                    (0x00, AccessSize::B8) => now,
                    _ => 0,
                }
            }
            MemOp::Write { .. }
            | MemOp::Writeback { .. }
            | MemOp::Maintain { .. }
            | MemOp::Prefetch { .. } => 0,
        }
    }
}

impl Device for GoldfishRtc {
    fn kind(&self) -> DeviceKind {
        DeviceKind::GoldfishRtc
    }

    fn region(&self) -> PhysRange {
        PhysRange::new(self.base_addr, crate::config::RTC_REGION_BYTES)
    }

    fn quiet_ticks(&self) -> Option<u64> {
        None
    }

    fn get_irq_id(&self) -> Option<IrqId> {
        Some(Self::IRQ)
    }
}
