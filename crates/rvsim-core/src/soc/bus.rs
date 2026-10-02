//! System interconnect (bus).
//!
//! Routes packets to MMIO devices or the memory controller, ticks devices,
//! folds CLINT and PLIC state into one set of interrupt lines per hart, and
//! says which addresses are plain RAM.

use super::devices::clint::Clint;
use super::devices::uart::Uart;
use super::devices::{Device, SimOp};
use crate::common::{HartId, LineAddr, PhysAddr};
use crate::sim::components::{ComponentId, DeviceId, MemCtrlId, ReqId};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::memory::GlobalMemory;
use crate::sim::packet::{HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData};
use std::collections::HashMap;

/// Interrupt lines presented to one hart, sampled by [`Bus::tick`] each cycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct HartIrqs {
    /// CLINT machine timer interrupt (`mip.MTIP`).
    pub mtip: bool,
    /// CLINT machine software interrupt (`mip.MSIP`).
    pub msip: bool,
    /// PLIC machine external interrupt (`mip.MEIP`).
    pub meip: bool,
    /// PLIC supervisor external interrupt (`mip.SEIP`).
    pub seip: bool,
}

/// The RAM range `[start, end)` and the memory controller serving it.
#[derive(Clone, Copy, Debug)]
struct RamWindow {
    ctrl: MemCtrlId,
    start: u64,
    end: u64,
}

/// System bus that routes packets to MMIO devices or the memory controller.
pub struct Bus {
    /// Registered MMIO devices.
    devices: Vec<Box<dyn Device + Send + Sync>>,
    /// Bus width in bytes (e.g., 8 for 64-bit); used to compute transfer cycles.
    pub width_bytes: u64,
    /// Base latency in cycles per transaction.
    pub latency_cycles: u64,
    uart_idx: Option<usize>,
    clint_idx: Option<usize>,
    plic_idx: Option<usize>,
    /// Interrupt lines per hart as of the last [`Bus::tick`].
    hart_irqs: Vec<HartIrqs>,
    /// The RAM range and the memory controller that serves it.
    ram: Option<RamWindow>,
    /// HTIF address range, checked before the RAM fast path so HTIF tohost
    /// stores route through the device.
    htif_range: Option<(u64, u64)>,
    /// In-flight RAM `MemReq`s forwarded to the memory controller, keyed by
    /// `ReqId` so the matching `MemResp` from the controller can be routed back
    /// to the originating upstream component (typically the LLC).
    pending: HashMap<ReqId, (ComponentId, usize)>,
    /// Cycle the request channel is free again: each transaction occupies
    /// it for its transfer time, so back-to-back requests queue.
    request_busy_until: u64,
    /// Cycle the response channel is free again.
    response_busy_until: u64,
}

impl std::fmt::Debug for Bus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bus")
            .field("width_bytes", &self.width_bytes)
            .field("latency_cycles", &self.latency_cycles)
            .field("uart_idx", &self.uart_idx)
            .field("clint_idx", &self.clint_idx)
            .field("num_devices", &self.devices.len())
            .field("ram", &self.ram)
            .finish_non_exhaustive()
    }
}

impl Bus {
    /// Creates a bus with the given width and latency serving `hart_count`
    /// harts' interrupt lines.
    pub fn new(width_bytes: u64, latency_cycles: u64, hart_count: usize) -> Self {
        Self {
            devices: Vec::new(),
            width_bytes,
            latency_cycles,
            uart_idx: None,
            clint_idx: None,
            plic_idx: None,
            hart_irqs: vec![HartIrqs::default(); hart_count],
            ram: None,
            htif_range: None,
            pending: HashMap::new(),
            request_busy_until: 0,
            response_busy_until: 0,
        }
    }

    /// Hands a host-side probe straight to the device mapped at its address,
    /// with no bus or device timing, so inspecting a device from outside the
    /// simulation leaves the run's timing untouched. Returns whether a
    /// device took it.
    pub fn probe_device(
        &mut self,
        packet: Packet,
        source: ComponentId,
        ctx: &mut HandleCtx<'_>,
    ) -> bool {
        let Packet::MemReq { paddr, .. } = &packet else { return false };
        let Some(idx) = self.find_device_idx(*paddr) else { return false };
        let device = DeviceId::new(u32::try_from(idx).unwrap_or(u32::MAX));
        self.handle_device(device, packet, source, ctx);
        true
    }

    /// Hands `packet` to the device `id` names, which sees itself as
    /// `ComponentId::Device(id)` for the requests it originates.
    pub fn handle_device(
        &mut self,
        id: DeviceId,
        packet: Packet,
        source: ComponentId,
        ctx: &mut HandleCtx<'_>,
    ) {
        let Some(device) = self.devices.get_mut(id.as_index()) else { return };
        let outer = ctx.self_id;
        ctx.self_id = ComponentId::Device(id);
        device.handle(packet, source, ctx);
        ctx.self_id = outer;
    }

    /// Registers a device on the bus; devices are sorted by base address for lookup.
    pub fn add_device(&mut self, dev: Box<dyn Device + Send + Sync>) {
        self.devices.push(dev);
        self.devices.sort_by_key(|d| d.address_range().0);
        self.uart_idx = self.devices.iter().position(|d| d.name() == "UART0");
        self.clint_idx = self.devices.iter().position(|d| d.name() == "CLINT");
        self.plic_idx = self.devices.iter().position(|d| d.name() == "PLIC");
        self.refresh_htif_range();
    }

    fn refresh_htif_range(&mut self) {
        self.htif_range = self.devices.iter().find(|d| d.name() == "HTIF").map(|d| {
            let (start, size) = d.address_range();
            (start, start + size)
        });
    }

    /// Tells the bus that `ctrl` serves the `size` bytes of RAM at `base`.
    pub const fn attach_ram(&mut self, ctrl: MemCtrlId, base: u64, size: u64) {
        self.ram = Some(RamWindow { ctrl, start: base, end: base + size });
    }

    /// True when `[paddr, paddr + size)` is plain RAM: inside the RAM range
    /// and clear of the HTIF window a device overlays on it.
    #[must_use]
    pub fn is_ram(&self, paddr: PhysAddr, size: u64) -> bool {
        let paddr = paddr.val();
        let Some(end) = paddr.checked_add(size) else { return false };
        self.ram.is_some_and(|ram| paddr >= ram.start && end <= ram.end)
            && !self.htif_range.is_some_and(|(start, stop)| paddr < stop && end > start)
    }

    /// Returns the cached `(start, end_exclusive)` HTIF range.
    #[inline]
    pub const fn htif_range(&self) -> Option<(u64, u64)> {
        self.htif_range
    }

    /// Returns cycles = base latency plus ceiling(bytes / `width_bytes`) transfers.
    pub const fn calculate_transit_time(&self, bytes: usize) -> u64 {
        self.latency_cycles + self.transfer_cycles(bytes)
    }

    /// Cycles a transaction of `bytes` occupies a channel.
    const fn transfer_cycles(&self, bytes: usize) -> u64 {
        (bytes as u64).div_ceil(self.width_bytes)
    }

    /// Claims the request channel from `now`, returning when the request
    /// has crossed the bus.
    const fn send_request(&mut self, now: u64, bytes: usize) -> u64 {
        let start = if now > self.request_busy_until { now } else { self.request_busy_until };
        self.request_busy_until = start + self.transfer_cycles(bytes);
        start + self.calculate_transit_time(bytes)
    }

    /// Claims the response channel from `now`, returning when the response
    /// has crossed the bus.
    const fn send_response(&mut self, now: u64, bytes: usize) -> u64 {
        let start = if now > self.response_busy_until { now } else { self.response_busy_until };
        self.response_busy_until = start + self.transfer_cycles(bytes);
        start + self.calculate_transit_time(bytes)
    }

    /// Returns whether the given physical address is backed by any device or RAM.
    pub fn is_valid_address(&self, paddr: PhysAddr) -> bool {
        let raw = paddr.val();
        if self.ram.is_some_and(|ram| raw >= ram.start && raw < ram.end) {
            return true;
        }
        self.devices.iter().any(|dev| {
            let (start, size) = dev.address_range();
            raw >= start && raw < start + size
        })
    }

    /// Number of harts whose interrupt lines this bus drives.
    #[must_use]
    pub const fn hart_count(&self) -> usize {
        self.hart_irqs.len()
    }

    /// Interrupt lines for `hart` as sampled by the last [`Bus::tick`].
    #[must_use]
    /// Requests the guest made of the simulator since the last call.
    pub fn take_sim_ops(&mut self) -> Vec<SimOp> {
        self.devices.iter_mut().flat_map(|d| d.take_sim_ops()).collect()
    }

    /// Finishes every device's work in flight before a checkpoint.
    pub fn drain_devices(&mut self, memory: &mut GlobalMemory) {
        for device in &mut self.devices {
            device.drain(memory);
        }
    }

    /// Every device's checkpoint state, keyed by device name.
    #[must_use]
    pub fn checkpoint_devices(&self) -> serde_json::Value {
        let states: serde_json::Map<String, serde_json::Value> = self
            .devices
            .iter()
            .filter_map(|device| Some((device.name().to_owned(), device.checkpoint()?)))
            .collect();
        serde_json::Value::Object(states)
    }

    /// Restores the devices named in `states`, as [`Bus::checkpoint_devices`]
    /// produced them.
    /// Checks that every device would take its state from `states`.
    ///
    /// # Errors
    ///
    /// Describes the first device that would not.
    pub fn check_device_states(&self, states: &serde_json::Value) -> Result<(), String> {
        for device in &self.devices {
            if let Some(state) = states.get(device.name()) {
                device.check_restore(state)?;
            }
        }
        Ok(())
    }

    /// Restores every device's state from `states`.
    ///
    /// # Errors
    ///
    /// Describes the first device that cannot take its state.
    pub fn restore_devices(&mut self, states: &serde_json::Value) -> Result<(), String> {
        for device in &mut self.devices {
            if let Some(state) = states.get(device.name()) {
                device.restore(state)?;
            }
        }
        Ok(())
    }

    /// The CLINT's `mtime`, which the `time` CSR reads; zero without a CLINT.
    #[must_use]
    pub fn mtime(&self) -> u64 {
        self.clint_idx.and_then(|i| self.devices[i].as_clint()).map_or(0, Clint::mtime)
    }

    /// Ticks the whole bus can skip; see [`Device::quiet_ticks`]. `None`
    /// when no device needs another tick until software touches one.
    pub fn quiet_ticks(&self) -> Option<u64> {
        self.devices.iter().filter_map(|device| device.quiet_ticks()).min()
    }

    /// Advances every device by `ticks` quiet ticks.
    pub fn skip_ticks(&mut self, ticks: u64) {
        for device in &mut self.devices {
            device.skip_ticks(ticks);
        }
    }

    /// Ticks until the CLINT's `mtime` reaches `value`, or `None` if it
    /// already has or there is no CLINT.
    pub fn ticks_until_mtime(&self, value: u64) -> Option<u64> {
        self.clint_idx.and_then(|i| self.devices[i].as_clint())?.ticks_until_mtime(value)
    }

    /// The interrupt lines currently asserted towards `hart`.
    pub fn hart_irqs(&self, hart: HartId) -> HartIrqs {
        self.hart_irqs.get(hart.as_index()).copied().unwrap_or_default()
    }

    /// Advances all devices by one tick, feeds the PLIC, and samples every
    /// hart's interrupt lines (read back with [`Bus::hart_irqs`]).
    pub fn tick(&mut self) {
        let mut active_irqs = 0u64;

        for dev in &mut self.devices {
            if dev.tick()
                && let Some(id) = dev.get_irq_id()
                && id.val() < 64
            {
                active_irqs |= 1 << id.val();
            }
        }

        let Self { devices, hart_irqs, clint_idx, plic_idx, .. } = self;
        for lines in hart_irqs.iter_mut() {
            *lines = HartIrqs::default();
        }

        if let Some(clint) = clint_idx.and_then(|idx| devices[idx].as_clint_mut()) {
            for (index, lines) in hart_irqs.iter_mut().enumerate() {
                let hart = HartId::new(u32::try_from(index).unwrap_or(u32::MAX));
                lines.mtip = clint.timer_pending(hart);
                lines.msip = clint.msip_pending(hart);
            }
        }

        if let Some(plic) = plic_idx.and_then(|idx| devices[idx].as_plic_mut()) {
            plic.update_irqs(active_irqs);
            plic.check_interrupts();
            for (index, lines) in hart_irqs.iter_mut().enumerate() {
                let hart = HartId::new(u32::try_from(index).unwrap_or(u32::MAX));
                let external = plic.hart_lines(hart);
                lines.meip = external.meip;
                lines.seip = external.seip;
            }
        }
    }

    /// The console UART.
    pub fn uart_mut(&mut self) -> Option<&mut Uart> {
        self.devices.get_mut(self.uart_idx?)?.as_uart_mut()
    }

    /// Whether a captured console holds output the host has not taken.
    pub fn console_has_output(&mut self) -> bool {
        self.uart_mut().is_some_and(|uart| uart.has_output())
    }

    /// Returns whether the UART device has detected a kernel panic pattern.
    pub fn check_kernel_panic(&mut self) -> bool {
        self.uart_mut().is_some_and(Uart::check_kernel_panic)
    }

    fn find_device_idx(&self, paddr: PhysAddr) -> Option<usize> {
        let raw = paddr.val();
        // HTIF is checked first because its range overlaps RAM.
        if let Some((start, end)) = self.htif_range
            && raw >= start
            && raw < end
        {
            return self.devices.iter().position(|d| d.name() == "HTIF");
        }
        self.devices.iter().position(|dev| {
            let (start, size) = dev.address_range();
            raw >= start && raw < start + size
        })
    }
}

/// Bytes an 8-byte address and its control take on the bus.
const COMMAND_BYTES: usize = 8;

/// Bytes a `MemReq` moves over the request channel: the command, plus the
/// data a write carries.
fn request_bytes(packet: &Packet) -> usize {
    match packet {
        Packet::MemReq { op: MemOp::Write { data: WriteData::Small(_), .. }, size, .. } => {
            COMMAND_BYTES + size.bytes()
        }
        Packet::MemReq { op: MemOp::Write { data: WriteData::Line { bytes, .. }, .. }, .. } => {
            COMMAND_BYTES + bytes.len()
        }
        Packet::MemReq {
            op: MemOp::Writeback { .. } | MemOp::Maintain { dirty: true, .. },
            size,
            ..
        } => COMMAND_BYTES + size.bytes(),
        _ => COMMAND_BYTES,
    }
}

/// Bytes the answer to `packet` moves over the response channel: the data
/// a read returns, or a bare acknowledgement for a write.
const fn response_bytes(packet: &Packet) -> usize {
    match packet {
        Packet::MemReq {
            op: MemOp::Write { .. } | MemOp::Writeback { .. } | MemOp::Maintain { .. },
            ..
        } => COMMAND_BYTES,
        Packet::MemReq { size, .. } => size.bytes(),
        _ => COMMAND_BYTES,
    }
}

impl Handle for Bus {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        match packet {
            Packet::MemReq { req_id, paddr, .. } => {
                let raw = paddr.val();
                let ram_hit = self.ram.filter(|ram| raw >= ram.start && raw < ram.end);
                let is_htif =
                    self.htif_range.is_some_and(|(hstart, hend)| raw >= hstart && raw < hend);
                if let Some(ram) = ram_hit
                    && !is_htif
                {
                    let _ = self.pending.insert(req_id, (source, response_bytes(&packet)));
                    let arrives = self.send_request(ctx.cycle, request_bytes(&packet));
                    ctx.scheduler.schedule(
                        arrives,
                        ComponentId::MemCtrl(ram.ctrl),
                        ctx.self_id,
                        packet,
                    );
                    return;
                }
                if let Some(idx) = self.find_device_idx(paddr) {
                    // The request crosses the bus to the device, which answers
                    // the bus after its access latency; the answer then
                    // crosses back (the `MemResp` arm).
                    let _ = self.pending.insert(req_id, (source, response_bytes(&packet)));
                    let arrives = self.send_request(ctx.cycle, request_bytes(&packet));
                    let device = DeviceId::new(u32::try_from(idx).unwrap_or(u32::MAX));
                    ctx.scheduler.schedule(
                        arrives,
                        ComponentId::Device(device),
                        ctx.self_id,
                        packet,
                    );
                    return;
                }
                // Unmapped address: reply with zeros so the originator unblocks.
                let line_addr = LineAddr::from_phys(paddr, 64);
                let arrives = self.send_response(ctx.cycle, response_bytes(&packet));
                ctx.scheduler.schedule(
                    arrives,
                    source,
                    ctx.self_id,
                    Packet::MemResp {
                        req_id,
                        line_addr,
                        data: MemRespData::Small(0),
                        hit_level: HitLevel::Mmio,
                        state: MesiState::Exclusive,
                    },
                );
            }
            Packet::MemResp { req_id, line_addr, data, hit_level, state } => {
                let Some((upstream, bytes)) = self.pending.remove(&req_id) else { return };
                let arrives = self.send_response(ctx.cycle, bytes);
                ctx.scheduler.schedule(
                    arrives,
                    upstream,
                    ctx.self_id,
                    Packet::MemResp { req_id, line_addr, data, hit_level, state },
                );
            }
            _ => {}
        }
    }
}
