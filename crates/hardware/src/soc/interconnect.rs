//! System interconnect (bus).
//!
//! Routes packets to MMIO devices or the memory controller, ticks devices,
//! aggregates IRQs through PLIC, and exposes a fast-path RAM region pointer
//! for pipeline bit-exact reads.

use super::devices::Device;
use super::memory::RamRegion;
use crate::common::{LineAddr, PhysAddr};
use crate::sim::components::{ComponentId, MemCtrlId, ReqId};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{HitLevel, MemRespData, Packet};
use std::collections::HashMap;

/// Aggregated interrupt signals returned by [`Bus::tick`] each cycle.
#[derive(Clone, Copy, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct BusIrqs {
    /// CLINT machine timer interrupt (`mip.MTIP`).
    pub timer: bool,
    /// CLINT machine software interrupt (`mip.MSIP`).
    pub msip: bool,
    /// PLIC machine external interrupt (`mip.MEIP`).
    pub meip: bool,
    /// PLIC supervisor external interrupt (`mip.SEIP`).
    pub seip: bool,
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
    /// Memory controller target for RAM-range accesses.
    ram_ctrl: Option<(MemCtrlId, u64, u64)>,
    /// Fast-path view of the DRAM region for bit-exact pipeline reads
    /// (instruction fetch, direct-mode loads).
    ram_region: Option<RamRegion>,
    /// HTIF address range, checked before the RAM fast path so HTIF tohost
    /// stores route through the device.
    htif_range: Option<(u64, u64)>,
    /// In-flight RAM `MemReq`s forwarded to the memory controller, keyed by
    /// `ReqId` so the matching `MemResp` from the controller can be routed back
    /// to the originating upstream component (typically the LLC).
    pending: HashMap<ReqId, ComponentId>,
}

impl std::fmt::Debug for Bus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bus")
            .field("width_bytes", &self.width_bytes)
            .field("latency_cycles", &self.latency_cycles)
            .field("uart_idx", &self.uart_idx)
            .field("clint_idx", &self.clint_idx)
            .field("num_devices", &self.devices.len())
            .field("ram_ctrl", &self.ram_ctrl)
            .finish_non_exhaustive()
    }
}

impl Bus {
    /// Creates a new bus with the given width and latency.
    pub fn new(width_bytes: u64, latency_cycles: u64) -> Self {
        Self {
            devices: Vec::new(),
            width_bytes,
            latency_cycles,
            uart_idx: None,
            clint_idx: None,
            ram_ctrl: None,
            ram_region: None,
            htif_range: None,
            pending: HashMap::new(),
        }
    }

    /// Registers a device on the bus; devices are sorted by base address for lookup.
    pub fn add_device(&mut self, dev: Box<dyn Device + Send + Sync>) {
        self.devices.push(dev);
        self.devices.sort_by_key(|d| d.address_range().0);
        self.uart_idx = self.devices.iter().position(|d| d.name() == "UART0");
        self.clint_idx = self.devices.iter().position(|d| d.name() == "CLINT");
        self.refresh_htif_range();
    }

    fn refresh_htif_range(&mut self) {
        self.htif_range = self.devices.iter().find(|d| d.name() == "HTIF").map(|d| {
            let (start, size) = d.address_range();
            (start, start + size)
        });
    }

    /// Tells the bus which memory controller handles RAM-range accesses and
    /// the `RamRegion` fast-path view.
    pub const fn attach_ram(&mut self, ctrl_id: MemCtrlId, region: RamRegion) {
        self.ram_ctrl = Some((ctrl_id, region.base(), region.base() + region.size()));
        self.ram_region = Some(region);
    }

    /// Returns the cached fast-path view of the DRAM region.
    #[inline]
    pub const fn ram_region(&self) -> Option<RamRegion> {
        self.ram_region
    }

    /// Returns the RAM fast-path view only when `[paddr, paddr+size)` is pure
    /// RAM — i.e. does not overlap an MMIO overlay (HTIF lives inside the RAM
    /// range, so its bytes belong to the device, not to `RamRegion`).
    pub fn ram_region_for(&self, paddr: u64, size: u64) -> Option<RamRegion> {
        self.ram_region.filter(|r| {
            if !r.contains(paddr, size) {
                return false;
            }
            !self
                .htif_range
                .is_some_and(|(start, end)| paddr < end && paddr + size > start)
        })
    }

    /// Returns the cached `(start, end_exclusive)` HTIF range.
    #[inline]
    pub const fn htif_range(&self) -> Option<(u64, u64)> {
        self.htif_range
    }

    /// Returns cycles = base latency plus ceiling(bytes / `width_bytes`) transfers.
    pub const fn calculate_transit_time(&self, bytes: usize) -> u64 {
        let transfers = (bytes as u64).div_ceil(self.width_bytes);
        self.latency_cycles + transfers
    }

    /// Writes a binary blob into RAM at the given physical address.
    pub const fn load_binary_at(&mut self, data: &[u8], addr: PhysAddr) {
        if let Some(region) = self.ram_region
            && region.contains(addr.val(), data.len() as u64)
        {
            // SAFETY: contains() above confirms the range is in-bounds.
            unsafe {
                let base = region.ptr(addr.val());
                std::ptr::copy_nonoverlapping(data.as_ptr(), base, data.len());
            }
        }
    }

    /// Returns whether the given physical address is backed by any device or RAM.
    pub fn is_valid_address(&self, paddr: PhysAddr) -> bool {
        let raw = paddr.val();
        if let Some((_, start, end)) = self.ram_ctrl
            && raw >= start
            && raw < end
        {
            return true;
        }
        self.devices.iter().any(|dev| {
            let (start, size) = dev.address_range();
            raw >= start && raw < start + size
        })
    }

    /// Advances all devices by one tick and updates PLIC. Returns the
    /// aggregated interrupt vector for this cycle.
    pub fn tick(&mut self) -> BusIrqs {
        let mut timer = false;
        let mut active_irqs = 0u64;

        for i in 0..self.devices.len() {
            let dev = &mut self.devices[i];
            if dev.tick() {
                if let Some(id) = dev.get_irq_id()
                    && id.val() < 64
                {
                    active_irqs |= 1 << id.val();
                }
                if dev.name() == "CLINT" {
                    timer = true;
                }
            }
        }

        let msip = self
            .clint_idx
            .and_then(|idx| self.devices[idx].as_clint_mut())
            .is_some_and(|clint| clint.msip_pending());

        let (meip, seip) = self.find_plic().map_or((false, false), |plic| {
            plic.update_irqs(active_irqs);
            plic.check_interrupts()
        });

        BusIrqs { timer, msip, meip, seip }
    }

    /// Returns whether the UART device has detected a kernel panic pattern.
    pub fn check_kernel_panic(&mut self) -> bool {
        if let Some(idx) = self.uart_idx
            && idx < self.devices.len()
            && let Some(uart) = self.devices[idx].as_uart_mut()
        {
            return uart.check_kernel_panic();
        }
        false
    }

    fn find_plic(&mut self) -> Option<&mut crate::soc::devices::Plic> {
        self.devices.iter_mut().find_map(|d| d.as_plic_mut())
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

/// Bytes-on-the-bus for the address phase of any `MemReq` (the bus carries an
/// 8-byte address plus control). Pre-refactor `simulate_memory_access`
/// charged `calculate_transit_time(8)` here too.
const BUS_REQ_BYTES: usize = 8;

/// Bytes-on-the-bus for a `MemResp` returning a cache line. Sub-line responses
/// are still charged this size to match the pre-refactor cycle model, which
/// always pulled a full line for the demand miss.
const BUS_RESP_BYTES: usize = 64;

impl Handle for Bus {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        match packet {
            Packet::MemReq { req_id, paddr, .. } => {
                let raw = paddr.val();
                let ram_hit = self
                    .ram_ctrl
                    .filter(|(_, start, end)| raw >= *start && raw < *end);
                let is_htif = self
                    .htif_range
                    .is_some_and(|(hstart, hend)| raw >= hstart && raw < hend);
                let req_transit = self.calculate_transit_time(BUS_REQ_BYTES);
                let resp_transit = self.calculate_transit_time(BUS_RESP_BYTES);

                if let Some((ctrl_id, _, _)) = ram_hit
                    && !is_htif
                {
                    let _ = self.pending.insert(req_id, source);
                    ctx.scheduler.schedule(
                        ctx.cycle + req_transit,
                        ComponentId::MemCtrl(ctrl_id),
                        ctx.self_id,
                        packet,
                    );
                    return;
                }
                if let Some(idx) = self.find_device_idx(paddr) {
                    self.devices[idx].handle(packet, source, ctx);
                    return;
                }
                // Unmapped address: reply with zeros so the originator unblocks.
                let line_addr = LineAddr::from_phys(paddr, 64);
                ctx.scheduler.schedule(
                    ctx.cycle + resp_transit,
                    source,
                    ctx.self_id,
                    Packet::MemResp {
                        req_id,
                        line_addr,
                        data: MemRespData::Small(0),
                        hit_level: HitLevel::Mmio,
                    },
                );
            }
            Packet::MemResp { req_id, line_addr, data, hit_level } => {
                let Some(upstream) = self.pending.remove(&req_id) else { return };
                ctx.scheduler.schedule(
                    ctx.cycle + self.calculate_transit_time(BUS_RESP_BYTES),
                    upstream,
                    ctx.self_id,
                    Packet::MemResp { req_id, line_addr, data, hit_level },
                );
            }
            _ => {}
        }
    }
}
