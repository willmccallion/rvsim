//! Memory controllers: own a backing DRAM buffer and respond to `MemReq`
//! packets with `MemResp` after a model-determined latency.
//!
//! `SimpleController` is a fixed-latency model serialised on a bandwidth
//! (gem5's `SimpleMemory`). `DramController` tracks per-bank
//! row buffers, tRRD between activations, and periodic refresh. Both read /
//! write the underlying [`DramBuffer`] directly so the response carries actual
//! data.

use std::num::NonZeroU64;
use std::sync::Arc;

use crate::common::{LineAddr, PhysAddr};
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, HitLevel, MemOp, MemRespData, MesiState, Packet};
use crate::sim::state::global_memory::GlobalMemory;
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

/// The rate at which the simple controller moves data (gem5's
/// `SimpleMemory.bandwidth`): each request keeps it busy for the time its
/// bytes take, and requests that arrive while it is busy wait their turn.
#[derive(Clone, Copy, Debug)]
pub struct Bandwidth {
    bytes_per_second: NonZeroU64,
    clock_hz: u64,
}

impl Bandwidth {
    /// `bytes_per_second` at a core clock of `clock_hz`.
    #[must_use]
    pub const fn new(bytes_per_second: NonZeroU64, clock_hz: u64) -> Self {
        Self { bytes_per_second, clock_hz }
    }

    /// Cycles the controller is busy moving `bytes`, at least one.
    fn occupancy(self, bytes: u64) -> u64 {
        let ticks = u128::from(bytes) * u128::from(self.clock_hz);
        let per_second = u128::from(self.bytes_per_second.get());
        u64::try_from(ticks.div_ceil(per_second)).unwrap_or(u64::MAX).max(1)
    }
}

/// Fixed-latency memory controller backed by a [`DramBuffer`], serialised
/// on its bandwidth.
#[derive(Debug)]
pub struct SimpleController {
    buffer: Arc<DramBuffer>,
    base: PhysAddr,
    latency: u64,
    bandwidth: Bandwidth,
    busy_until: u64,
}

impl SimpleController {
    /// Creates a simple controller. `base` is the physical address at which the
    /// buffer's first byte is mapped.
    pub const fn new(
        buffer: Arc<DramBuffer>,
        base: PhysAddr,
        latency: u64,
        bandwidth: Bandwidth,
    ) -> Self {
        Self { buffer, base, latency, bandwidth, busy_until: 0 }
    }

    /// Returns a clone of the underlying DRAM buffer handle.
    pub fn buffer(&self) -> Arc<DramBuffer> {
        Arc::clone(&self.buffer)
    }
}

impl Handle for SimpleController {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            if is_dataless_maintenance(&op) {
                acknowledge_now(req_id, paddr, source, ctx);
                return;
            }
            let data = service_request(&self.buffer, self.base, paddr, size, &op, ctx.memory);
            let started = ctx.cycle.max(self.busy_until);
            self.busy_until = started + self.bandwidth.occupancy(size.bytes() as u64);
            ctx.scheduler.schedule(
                started + self.latency,
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
            if is_dataless_maintenance(&op) {
                acknowledge_now(req_id, paddr, source, ctx);
                return;
            }
            let latency = self.compute_latency(paddr.val(), ctx.cycle);
            let data = service_request(&self.buffer, self.base, paddr, size, &op, ctx.memory);
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

    /// Continues from simulator cycle `cycle` after a checkpoint restore,
    /// as a controller powered up then would: no timing history, and
    /// refreshes scheduled from that cycle. Queued requests are kept.
    fn resume_at(&mut self, cycle: u64);
}

impl MemoryController for SimpleController {
    fn resume_at(&mut self, _cycle: u64) {
        self.busy_until = 0;
    }
}

impl MemoryController for DramController {
    fn resume_at(&mut self, cycle: u64) {
        for bank in &mut self.banks {
            *bank = BankState { open_row: None, busy_until: 0 };
        }
        self.last_activate_cycle = None;
        self.next_refresh_cycle = if self.t_refi > 0 { cycle + self.t_refi } else { u64::MAX };
    }
}

/// Serves a request at the controller: a hart's access takes effect in
/// `memory` now; a line read returns the line.
fn service_request(
    buffer: &Arc<DramBuffer>,
    base: PhysAddr,
    paddr: PhysAddr,
    size: AccessSize,
    op: &MemOp,
    memory: &mut GlobalMemory,
) -> MemRespData {
    if op.takes_effect_when_served(size) {
        return memory.perform(paddr, size, op);
    }
    let offset = (paddr.val().saturating_sub(base.val())) as usize;
    match op {
        MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. } => {
            read_response(buffer, offset, size)
        }
        MemOp::Write { .. } | MemOp::Writeback { .. } | MemOp::Maintain { .. } => {
            MemRespData::Small(0)
        }
    }
}

/// Answers `req_id` in this cycle.
fn acknowledge_now(req_id: ReqId, paddr: PhysAddr, source: ComponentId, ctx: &mut HandleCtx<'_>) {
    ctx.scheduler.schedule(
        ctx.cycle,
        source,
        ctx.self_id,
        Packet::MemResp {
            req_id,
            line_addr: LineAddr::from_phys(paddr, CACHE_LINE_BYTES),
            data: MemRespData::Small(0),
            hit_level: HitLevel::Dram,
            state: MesiState::Exclusive,
        },
    );
}

/// True for a maintenance operation that brings no data: memory, the point
/// of coherence, acknowledges it as it arrives without a DRAM access. One
/// carrying dirty data is a line write.
const fn is_dataless_maintenance(op: &MemOp) -> bool {
    matches!(op, MemOp::Maintain { dirty: false, .. })
}

fn read_response(buffer: &Arc<DramBuffer>, offset: usize, size: AccessSize) -> MemRespData {
    if size == AccessSize::Line {
        let s = buffer.read_slice(offset, CACHE_LINE_BYTES as usize);
        return MemRespData::Line(s.to_vec().into_boxed_slice());
    }
    let bytes = buffer.read_slice(offset, size.bytes());
    MemRespData::Small(bytes.iter().rev().fold(0, |value, &byte| (value << 8) | u64::from(byte)))
}
