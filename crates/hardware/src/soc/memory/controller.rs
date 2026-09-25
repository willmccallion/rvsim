//! Memory controllers: own a backing DRAM buffer and respond to `MemReq`
//! packets with `MemResp` after a model-determined latency.
//!
//! `SimpleController` is a fixed-latency model. `DramController` tracks per-bank
//! row buffers, tRRD between activations, and periodic refresh. Both read /
//! write the underlying [`DramBuffer`] directly so the response carries actual
//! data.

use std::sync::Arc;

use crate::common::{LineAddr, PhysAddr};
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, HitLevel, MemOp, MemRespData, Packet, MesiState};
use crate::soc::memory::buffer::DramBuffer;

/// Cache-line size used when building `LineAddr` from a `PhysAddr`.
const CACHE_LINE_BYTES: u64 = 64;

/// Configuration parameters for constructing a [`DramController`].
#[derive(Clone, Copy, Debug)]
pub struct DramConfig {
    /// Column access strobe latency (cycles).
    pub t_cas: u64,
    /// Row access strobe latency (cycles).
    pub t_ras: u64,
    /// Precharge latency (cycles).
    pub t_pre: u64,
    /// Row-to-row delay for different-bank activations (cycles).
    pub t_rrd: u64,
    /// Number of independent DRAM banks.
    pub num_banks: usize,
    /// Size of a DRAM row (page) in bytes. Must be a power of two.
    pub row_size_bytes: usize,
    /// Refresh interval in cycles (0 disables refresh).
    pub t_refi: u64,
    /// Refresh cycle time in cycles.
    pub t_rfc: u64,
}

/// Per-bank state for DRAM row-buffer tracking.
#[derive(Debug)]
struct BankState {
    /// Currently open row in this bank, or `None` if no row is active.
    open_row: Option<u64>,
    /// Cycle at which this bank becomes available (after activation or refresh).
    busy_until: u64,
}

/// Fixed-latency memory controller backed by a [`DramBuffer`].
#[derive(Debug)]
pub struct SimpleController {
    buffer: Arc<DramBuffer>,
    base: PhysAddr,
    latency: u64,
}

impl SimpleController {
    /// Creates a simple controller. `base` is the physical address at which the
    /// buffer's first byte is mapped.
    pub const fn new(buffer: Arc<DramBuffer>, base: PhysAddr, latency: u64) -> Self {
        Self { buffer, base, latency }
    }

    /// Returns a clone of the underlying DRAM buffer handle.
    pub fn buffer(&self) -> Arc<DramBuffer> {
        Arc::clone(&self.buffer)
    }
}

impl Handle for SimpleController {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let data = service_request(&self.buffer, self.base, paddr, size, &op);
            ctx.scheduler.schedule(
                ctx.cycle + self.latency,
                source,
                ctx.self_id,
                Packet::MemResp {
                    req_id,
                    line_addr: LineAddr::from_phys(paddr, CACHE_LINE_BYTES),
                    data,
                    hit_level: HitLevel::Dram,
                state: MesiState::Exclusive,
                },
            );
        }
    }
}

/// DRAM controller with multi-bank row buffers, tRRD, and refresh modeling.
///
/// Each bank independently tracks its open row and busy state. Refresh
/// periodically marks all banks as unavailable for `t_rfc` cycles.
#[derive(Debug)]
pub struct DramController {
    buffer: Arc<DramBuffer>,
    base: PhysAddr,
    banks: Vec<BankState>,
    num_banks: usize,
    t_cas: u64,
    t_ras: u64,
    t_pre: u64,
    t_rrd: u64,
    t_refi: u64,
    t_rfc: u64,
    row_mask: u64,
    row_shift: u32,
    /// Cycle of the last bank activation (for tRRD enforcement).
    last_activate_cycle: Option<u64>,
    /// Next cycle at which an auto-refresh fires.
    next_refresh_cycle: u64,
}

impl DramController {
    /// Creates a DRAM controller from a [`DramConfig`].
    pub fn new(buffer: Arc<DramBuffer>, base: PhysAddr, cfg: DramConfig) -> Self {
        debug_assert!(
            cfg.row_size_bytes.is_power_of_two(),
            "row_size_bytes must be a power of two"
        );
        debug_assert!(cfg.num_banks > 0, "num_banks must be > 0");

        let row_shift = cfg.row_size_bytes.trailing_zeros();
        let row_mask = !(cfg.row_size_bytes as u64 - 1);

        let mut banks = Vec::with_capacity(cfg.num_banks);
        for _ in 0..cfg.num_banks {
            banks.push(BankState { open_row: None, busy_until: 0 });
        }

        Self {
            buffer,
            base,
            banks,
            num_banks: cfg.num_banks,
            t_cas: cfg.t_cas,
            t_ras: cfg.t_ras,
            t_pre: cfg.t_pre,
            t_rrd: cfg.t_rrd,
            t_refi: cfg.t_refi,
            t_rfc: cfg.t_rfc,
            row_mask,
            row_shift,
            last_activate_cycle: None,
            next_refresh_cycle: if cfg.t_refi > 0 { cfg.t_refi } else { u64::MAX },
        }
    }

    /// Returns a clone of the underlying DRAM buffer handle.
    pub fn buffer(&self) -> Arc<DramBuffer> {
        Arc::clone(&self.buffer)
    }

    #[inline]
    const fn bank_index(&self, addr: u64) -> usize {
        ((addr >> self.row_shift) as usize) % self.num_banks
    }

    #[inline]
    const fn row_addr(&self, addr: u64) -> u64 {
        addr & self.row_mask
    }

    fn handle_refresh(&mut self, current_cycle: u64) -> u64 {
        if self.t_refi == 0 {
            return current_cycle;
        }

        let mut effective_cycle = current_cycle;

        while effective_cycle >= self.next_refresh_cycle {
            let refresh_end = self.next_refresh_cycle + self.t_rfc;
            for bank in &mut self.banks {
                if bank.busy_until < refresh_end {
                    bank.busy_until = refresh_end;
                }
                bank.open_row = None;
            }
            self.next_refresh_cycle += self.t_refi;
            if effective_cycle < refresh_end {
                effective_cycle = refresh_end;
            }
        }

        effective_cycle
    }

    const fn activate(&mut self, mut ready_cycle: u64) -> u64 {
        if let Some(last_act) = self.last_activate_cycle {
            let earliest_activate = last_act + self.t_rrd;
            if ready_cycle < earliest_activate {
                ready_cycle = earliest_activate;
            }
        }
        self.last_activate_cycle = Some(ready_cycle);
        ready_cycle
    }

    /// Computes the latency in cycles for an access at `addr` starting at
    /// `current_cycle`. Mutates bank state and refresh tracking.
    fn compute_latency(&mut self, addr: u64, current_cycle: u64) -> u64 {
        let mut ready_cycle = self.handle_refresh(current_cycle);

        let bank_idx = self.bank_index(addr);
        let row = self.row_addr(addr);

        if ready_cycle < self.banks[bank_idx].busy_until {
            ready_cycle = self.banks[bank_idx].busy_until;
        }

        match self.banks[bank_idx].open_row {
            Some(open_row) if open_row == row => {
                self.banks[bank_idx].busy_until = ready_cycle + self.t_cas;
                (ready_cycle - current_cycle) + self.t_cas
            }
            Some(_) => {
                ready_cycle += self.t_pre;
                ready_cycle = self.activate(ready_cycle);
                self.banks[bank_idx].open_row = Some(row);
                self.banks[bank_idx].busy_until = ready_cycle + self.t_ras;
                (ready_cycle - current_cycle) + self.t_ras + self.t_cas
            }
            None => {
                ready_cycle = self.activate(ready_cycle);
                self.banks[bank_idx].open_row = Some(row);
                self.banks[bank_idx].busy_until = ready_cycle + self.t_ras;
                (ready_cycle - current_cycle) + self.t_ras + self.t_cas
            }
        }
    }
}

impl Handle for DramController {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let latency = self.compute_latency(paddr.val(), ctx.cycle);
            let data = service_request(&self.buffer, self.base, paddr, size, &op);
            ctx.scheduler.schedule(
                ctx.cycle + latency,
                source,
                ctx.self_id,
                Packet::MemResp {
                    req_id,
                    line_addr: LineAddr::from_phys(paddr, CACHE_LINE_BYTES),
                    data,
                    hit_level: HitLevel::Dram,
                state: MesiState::Exclusive,
                },
            );
        }
    }
}

/// Pluggable memory controller. Any type that implements [`Handle`] and is
/// `Send + Sync` can be dropped into `SimState::mem_controller` as a
/// `Box<dyn MemoryController + Send + Sync>`.
///
/// [`Self::tick`] is invoked once per simulator cycle. Controllers that
/// service requests synchronously in `handle` (fixed-latency [`SimpleController`],
/// per-request [`DramController`]) get the default no-op tick. The DDR5
/// controller uses tick to advance its per-cycle command scheduler.
pub trait MemoryController: Handle + Send + Sync + std::fmt::Debug {
    /// Advances the controller by one simulator cycle. Default implementation
    /// is a no-op for controllers that do all their work in `handle`.
    fn tick(&mut self, _ctx: &mut HandleCtx<'_>) {}
}

impl MemoryController for SimpleController {}
impl MemoryController for DramController {}

/// Reads or writes the underlying buffer for a memory request and returns the
/// response payload.
///
/// Writes do not touch the backing buffer here: scalar/vector store drains
/// at commit and CBO routines write the bytes into `RamRegion` directly so
/// subsequent loads via the RAM fast path see the new value immediately. The
/// `MemReq` packet still flows through the cache + bus + controller chain
/// for cache-state and latency accounting; this function returns a zero ack
/// payload for the response.
fn service_request(
    buffer: &Arc<DramBuffer>,
    base: PhysAddr,
    paddr: PhysAddr,
    size: AccessSize,
    op: &MemOp,
) -> MemRespData {
    let offset = (paddr.val().saturating_sub(base.val())) as usize;
    match op {
        MemOp::Read | MemOp::ReadOwn | MemOp::Fetch => read_response(buffer, offset, size),
        MemOp::Write { .. } | MemOp::Writeback { .. } => MemRespData::Small(0),
        MemOp::Atomic { .. } => {
            // Atomic semantics are resolved upstream (LR/SC reservation, AMO
            // round-trip in the LSU). The controller serves the load value.
            read_response(buffer, offset, size)
        }
    }
}

fn read_response(buffer: &Arc<DramBuffer>, offset: usize, size: AccessSize) -> MemRespData {
    match size {
        AccessSize::B1 => MemRespData::Small(u64::from(buffer.read_u8(offset))),
        AccessSize::B2 => {
            let s = buffer.read_slice(offset, 2);
            MemRespData::Small(u64::from(u16::from_le_bytes([s[0], s[1]])))
        }
        AccessSize::B4 => {
            let s = buffer.read_slice(offset, 4);
            MemRespData::Small(u64::from(u32::from_le_bytes([s[0], s[1], s[2], s[3]])))
        }
        AccessSize::B8 => {
            let s = buffer.read_slice(offset, 8);
            MemRespData::Small(u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
        }
        AccessSize::Line => {
            let s = buffer.read_slice(offset, CACHE_LINE_BYTES as usize);
            MemRespData::Line(s.to_vec().into_boxed_slice())
        }
    }
}

