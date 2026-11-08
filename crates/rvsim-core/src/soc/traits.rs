//! `Device` trait implemented by all bus-attached MMIO components.
//!
//! Data-bearing operations (reads / writes) happen via the
//! [`crate::sim::handle::Handle`] trait, which every device implements.
//! `Device` itself describes the device's location on the bus, its lifecycle
//! tick, and type-specific upcasts that the bus uses for IRQ aggregation.

use crate::common::{IrqId, LineAddr, PhysAddr};
use crate::config::DeviceKind;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::memory::GlobalMemory;
use crate::sim::packet::{AccessSize, HitLevel, MemOp, MemRespData, MesiState, Packet};
use serde::de::DeserializeOwned;
use std::any::Any;

/// The line size an MMIO response's address is rounded to. Devices have no
/// cache lines; the requester matches the response by its id.
const MMIO_LINE_BYTES: u64 = 64;

/// A device whose registers MMIO reads and writes reach.
pub trait MmioRegisters: Device {
    /// Performs `op` of `size` bytes at `offset` from the device's base,
    /// and returns what a read returns (0 for anything else).
    fn access(&mut self, offset: u64, size: AccessSize, op: MemOp, ctx: &mut HandleCtx<'_>) -> u64;
}

/// Serves an MMIO request: performs it on `device` and answers `source`
/// after the device's access latency. Anything but a request is ignored.
pub fn serve_mmio<D: MmioRegisters>(
    device: &mut D,
    packet: Packet,
    source: ComponentId,
    ctx: &mut HandleCtx<'_>,
) {
    let Packet::MemReq { req_id, paddr, size, op, .. } = packet else { return };
    let value = device.access(paddr.val().saturating_sub(device.address_range().0), size, op, ctx);
    let answered_at = ctx.cycle + ctx.config.system.device_access_cycles(device.kind());
    ctx.scheduler.schedule(answered_at, source, ctx.self_id, mmio_response(req_id, paddr, value));
}

/// The answer to MMIO request `req_id` for `paddr`, carrying `value`.
#[must_use]
pub const fn mmio_response(req_id: ReqId, paddr: PhysAddr, value: u64) -> Packet {
    Packet::MemResp {
        req_id,
        line_addr: LineAddr::from_phys(paddr, MMIO_LINE_BYTES),
        data: MemRespData::Small(value),
        hit_level: HitLevel::Mmio,
        state: MesiState::Exclusive,
    }
}

/// Why a device cannot take the state a checkpoint holds for it.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    /// The state is not in the device's checkpoint format.
    #[error("{} state: {source}", .device.name())]
    Malformed {
        /// The device.
        device: DeviceKind,
        /// What the state lacked or got wrong.
        source: serde_json::Error,
    },
    /// HTIF's program had host files open, which a restored run cannot
    /// get back.
    #[error(
        "HTIF: the program had {count} host files open at the checkpoint, which a restored run cannot get back"
    )]
    HostFilesOpen {
        /// How many.
        count: usize,
    },
    /// The disk image is not the one the checkpoint was taken on.
    #[error("the disk image differs from the one the checkpoint was taken on")]
    DiskImageDiffers,
    /// A written sector's contents are not hex.
    #[error("a written sector is not hex")]
    WrittenSectorNotHex,
    /// A written sector lies past the end of the disk.
    #[error("a written sector lies past the end of the disk")]
    WrittenSectorPastDisk,
}

/// `state` as `device`'s checkpoint type.
///
/// # Errors
///
/// [`RestoreError::Malformed`] when it is not in that format.
pub fn parse_state<T: DeserializeOwned>(
    device: DeviceKind,
    state: &serde_json::Value,
) -> Result<T, RestoreError> {
    T::deserialize(state).map_err(|source| RestoreError::Malformed { device, source })
}

/// Trait for memory-mapped I/O devices attached to the system bus.
///
/// All data accesses go through the device's [`Handle`] impl; this trait only
/// describes layout (name + address range), per-cycle lifecycle (`tick`), and
/// device-class upcasts the bus uses for routing IRQs and panic detection.
pub trait Device: Handle + Send + Sync + Any {
    /// Which device this is.
    fn kind(&self) -> DeviceKind;

    /// Its name, as configs, checkpoints and statistics spell it.
    fn name(&self) -> &'static str {
        self.kind().name()
    }
    /// Returns (`base_address`, `size_in_bytes`) for this device's MMIO region.
    fn address_range(&self) -> (u64, u64);

    /// Advances device state by one cycle; returns `true` if an IRQ was raised
    /// (e.g., timer).
    fn tick(&mut self) -> bool {
        false
    }
    /// How many of the coming ticks would change nothing but the device's
    /// own clock, or `None` when it will stay that way until software
    /// touches it. The default says none, so a device skips no time until
    /// it implements [`Device::skip_ticks`].
    fn quiet_ticks(&self) -> Option<u64> {
        Some(0)
    }

    /// Advances the device by `ticks` ticks, at most [`Device::quiet_ticks`],
    /// leaving it exactly as ticking would.
    fn skip_ticks(&mut self, _ticks: u64) {}

    /// Returns the IRQ ID for this device if it can raise interrupts.
    fn get_irq_id(&self) -> Option<IrqId> {
        None
    }

    /// Finishes the device's work in flight before a checkpoint, the way
    /// a pipeline drain writes its committed stores.
    fn drain(&mut self, _memory: &mut GlobalMemory) {}

    /// The device's architectural state for a checkpoint; `None` for a
    /// device with none.
    fn checkpoint(&self) -> Option<serde_json::Value> {
        None
    }

    /// Checks that [`Device::restore`] would take `state`, without
    /// changing anything, so a restore can fail before it starts.
    ///
    /// # Errors
    ///
    /// Why the state cannot be restored into this device.
    fn check_restore(&self, _state: &serde_json::Value) -> Result<(), RestoreError> {
        Ok(())
    }

    /// Restores state a [`Device::checkpoint`] produced.
    ///
    /// # Errors
    ///
    /// Why the state cannot be restored into this device.
    fn restore(&mut self, _state: &serde_json::Value) -> Result<(), RestoreError> {
        Ok(())
    }
}
