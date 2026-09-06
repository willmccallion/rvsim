//! Simulator: owns the state, the pipeline, and the global event queue.
//!
//! Each `tick()`:
//! 1. Increments the cycle in `pre_tick`.
//! 2. Drains events scheduled for the new cycle, delivering packets to
//!    pipelines / caches / bus / memory controllers.
//! 3. Runs one cycle of the pipeline (mailbox-drain at the top, then
//!    engine.tick, then frontend.tick).
//! 4. Drains again so packets the pipeline just emitted reach their
//!    targets this cycle. The cache / bus / mem-controller handlers
//!    schedule their responses for future cycles; those land in the
//!    pipeline's mailbox via the next cycle's start-of-tick drain.
//! 5. Runs `post_tick` for mode tracing.
//!
//! Memory traffic (instruction fetch, load, store, page-table walk) flows
//! exclusively through scheduled `MemReq` / `MemResp` packets.

use crate::common::SimError;
use crate::config::Config;
use crate::core::pipeline::backend::inorder::InOrderEngine;
use crate::core::pipeline::backend::o3::O3Engine;
use crate::core::pipeline::engine::{BackendType, Pipeline, PipelineDispatch};
use crate::core::pipeline::frontend::Frontend;
use crate::sim::components::{CacheId, ComponentId, MemCtrlId, PipelineId};
use crate::sim::events::Event;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::Packet;
use crate::sim::state::SimState;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

/// Default `CacheId` for the L1 instruction cache in a single-core config.
const L1I_CACHE_ID: CacheId = CacheId::new(0);
/// Default `CacheId` for the L1 data cache in a single-core config.
const L1D_CACHE_ID: CacheId = CacheId::new(1);
/// Default `PipelineId` for the single-core pipeline.
const PIPELINE_ID: PipelineId = PipelineId::new(0);

/// Top-level simulator: `SimState` + pipeline + scheduler.
#[derive(Debug)]
pub struct Simulator {
    /// Simulator-side architectural state (hart, core, bus, caches, stats).
    pub state: SimState,
    /// Pipeline implementation (frontend + backend engine).
    pub pipeline: PipelineDispatch,
}

unsafe impl Send for Simulator {}
unsafe impl Sync for Simulator {}

impl Simulator {
    /// Wraps an existing `SimState` with a pipeline built from its `config`.
    /// Use this when the caller needs to interleave setup between state
    /// construction and pipeline dispatch (e.g. loading an ELF image and
    /// registering HTIF before the pipeline reads the reset PC).
    pub fn new(state: SimState) -> Self {
        let config = &state.config;
        let pipeline = match config.pipeline.backend {
            BackendType::InOrder => PipelineDispatch::InOrder(Box::new(Pipeline {
                frontend: Frontend::new(config.pipeline.width),
                engine: InOrderEngine::new(config, PIPELINE_ID, L1I_CACHE_ID, L1D_CACHE_ID),
                rename_output: Vec::with_capacity(config.pipeline.width),
                redirect_pending: false,
            })),
            BackendType::OutOfOrder => PipelineDispatch::OutOfOrder(Box::new(Pipeline {
                frontend: Frontend::new(config.pipeline.width),
                engine: O3Engine::new(config, PIPELINE_ID, L1I_CACHE_ID, L1D_CACHE_ID),
                rename_output: Vec::with_capacity(config.pipeline.width),
                redirect_pending: false,
            })),
        };
        Self { state, pipeline }
    }

    /// Convenience constructor: builds the exit-signal `Arc`, the `SimState`,
    /// and the `Simulator` together. Use this when the caller doesn't need
    /// to touch the state between construction and pipeline start.
    pub fn build(config: &Config, disk_path: &str) -> Self {
        let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
        Self::new(SimState::new(config, disk_path, exit_signal))
    }

    /// Synchronize the architectural register file into the O3 PRF.
    ///
    /// Must be called after all register initialization (loader setup, etc.)
    /// but before the first pipeline tick. For the in-order backend this is a no-op.
    pub fn sync_arch_regs(&mut self) {
        if let PipelineDispatch::OutOfOrder(ref mut p) = self.pipeline {
            p.engine.sync_arch_regs(&self.state);
        }
    }

    /// Advances the simulator by one clock cycle.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::HangDetected`] if the PC has not advanced for too many
    /// consecutive cycles (and is not stuck in a WFI spin-wait).
    ///
    /// Returns [`SimError::KernelPanic`] if the guest OS panic sentinel fires.
    pub fn tick(&mut self) -> Result<(), SimError> {
        let prev_priv = self.state.hart.privilege;
        let skip = self.state.pre_tick()?;
        // First drain: deliver events scheduled for cycles <= now into
        // their targets (filling pipeline.mailbox with responses from
        // previous cycles' emissions).
        self.drain_events();
        if !skip {
            self.pipeline.tick(&mut self.state);
        }
        // Second drain: events the pipeline just scheduled (MemReqs to L1)
        // reach their target component handlers this cycle so the next
        // cycle's start-of-tick drain delivers their responses.
        self.drain_events();
        self.state.post_tick(prev_priv);
        Ok(())
    }

    /// Dispatches every event with `fire_at <= self.state.cycle`.
    fn drain_events(&mut self) {
        let cycle = self.state.cycle;
        while let Some(event) = self.state.event_queue.pop_ready(cycle) {
            self.dispatch(event);
        }
    }

    /// Routes a single event to its target component.
    fn dispatch(&mut self, event: Event) {
        let Event { fire_at: _, seq: _, target, source, packet } = event;
        match target {
            ComponentId::Pipeline(_) => {
                self.pipeline.deliver(source, packet);
            }
            ComponentId::Cache(id) => {
                dispatch_to_cache(&mut self.state, id, packet, source);
            }
            ComponentId::Bus => {
                let cycle = self.state.cycle;
                let mut ctx = HandleCtx {
                    scheduler: &mut self.state.event_queue,
                    stats: &mut self.state.stats_hier,
                    config: &self.state.config,
                    cycle,
                    self_id: ComponentId::Bus,
                };
                self.state.bus.handle(packet, source, &mut ctx);
            }
            ComponentId::MemCtrl(id) => {
                let cycle = self.state.cycle;
                let mut ctx = HandleCtx {
                    scheduler: &mut self.state.event_queue,
                    stats: &mut self.state.stats_hier,
                    config: &self.state.config,
                    cycle,
                    self_id: ComponentId::MemCtrl(id),
                };
                self.state.mem_controller.handle(packet, source, &mut ctx);
            }
            ComponentId::Device(_) | ComponentId::Hart(_) | ComponentId::Core(_) => {
                // Devices are routed via Bus; Hart / Core targeting is
                // reserved for future multi-core / coherence packets.
            }
        }
    }

    /// Retrieves the exit code if the simulation has finished.
    pub fn take_exit(&self) -> Option<u64> {
        self.state.take_exit()
    }

    /// Synchronously reads `width` bytes from physical memory.
    ///
    /// Used at the FFI boundary (Python bindings, save/restore tooling) to
    /// inspect memory without driving the full pipeline. RAM addresses use
    /// the fast-path pointer; MMIO addresses dispatch a `MemReq` through the
    /// bus's `Handle` impl with a local event queue and read the response
    /// data out of the synchronously-scheduled `MemResp`.
    ///
    /// Not for use inside pipeline stages — those emit `MemReq` packets
    /// through the global event queue and consume responses via the
    /// mailbox-drain stage.
    pub fn probe_mem_load(&mut self, paddr: crate::common::PhysAddr, width: u8) -> u64 {
        let raw = paddr.val();
        if let Some(r) = self
            .state
            .bus
            .ram_region()
            .filter(|r| r.contains(raw, u64::from(width)))
        {
            // SAFETY: bounds-checked by `RamRegion::contains(raw, width)`.
            return unsafe {
                match width {
                    1 => u64::from(*r.ptr(raw)),
                    2 => u64::from(r.ptr(raw).cast::<u16>().read_unaligned()),
                    4 => u64::from(r.ptr(raw).cast::<u32>().read_unaligned()),
                    8 => r.ptr(raw).cast::<u64>().read_unaligned(),
                    _ => 0,
                }
            };
        }
        self.probe_mmio(paddr, width, crate::sim::packet::MemOp::Read)
    }

    /// Synchronously writes `width` bytes to physical memory. For RAM the
    /// fast-path pointer is used directly; for MMIO a `MemReq` is dispatched
    /// through the bus's `Handle` impl so the device's side effect runs.
    pub fn probe_mem_store(
        &mut self,
        paddr: crate::common::PhysAddr,
        value: u64,
        width: u8,
    ) {
        let raw = paddr.val();
        if let Some(r) = self
            .state
            .bus
            .ram_region()
            .filter(|r| r.contains(raw, u64::from(width)))
        {
            // SAFETY: bounds-checked above.
            unsafe {
                match width {
                    1 => *r.ptr(raw) = value as u8,
                    2 => r.ptr(raw).cast::<u16>().write_unaligned(value as u16),
                    4 => r.ptr(raw).cast::<u32>().write_unaligned(value as u32),
                    8 => r.ptr(raw).cast::<u64>().write_unaligned(value),
                    _ => {}
                }
            }
            return;
        }
        let op = crate::sim::packet::MemOp::Write {
            data: crate::sim::packet::WriteData::Small(value),
        };
        let _ = self.probe_mmio(paddr, width, op);
    }

    /// Internal helper: synchronously dispatches an MMIO `MemReq` through the
    /// bus and reads the response data out of a local event queue.
    fn probe_mmio(
        &mut self,
        paddr: crate::common::PhysAddr,
        width: u8,
        op: crate::sim::packet::MemOp,
    ) -> u64 {
        use crate::sim::components::{ComponentId, PipelineId, ReqId};
        use crate::sim::events::EventQueue;
        use crate::sim::handle::Handle;
        use crate::sim::handle::HandleCtx;
        use crate::sim::packet::{AccessSize, MemRespData, Packet};
        use crate::sim::stats::Stats;

        let access_size = match width {
            1 => AccessSize::B1,
            2 => AccessSize::B2,
            4 => AccessSize::B4,
            _ => AccessSize::B8,
        };
        let req_id = ReqId::new(u64::MAX);
        let mut local_queue = EventQueue::new();
        let mut local_stats = Stats::new();
        let cycle = self.state.cycle;
        let mut ctx = HandleCtx {
            scheduler: &mut local_queue,
            stats: &mut local_stats,
            config: &self.state.config,
            cycle,
            self_id: ComponentId::Bus,
        };
        self.state.bus.handle(
            Packet::MemReq {
                req_id,
                paddr,
                vaddr: None,
                size: access_size,
                op,
            },
            ComponentId::Pipeline(PipelineId::new(0)),
            &mut ctx,
        );
        while let Some(event) = local_queue.pop_ready(u64::MAX) {
            if let Packet::MemResp { req_id: rid, data, .. } = event.packet
                && rid == req_id
            {
                return match data {
                    MemRespData::Small(v) => v,
                    MemRespData::Line(_) => 0,
                };
            }
        }
        0
    }
}

/// Dispatches a packet to the cache identified by `id`.
fn dispatch_to_cache(state: &mut SimState, id: CacheId, packet: Packet, source: ComponentId) {
    let self_id = ComponentId::Cache(id);
    let cycle = state.cycle;
    // Split-borrow SimState fields explicitly so the HandleCtx (borrowing
    // event_queue / stats_hier / config) coexists with the cache field
    // borrow.
    let scheduler = &mut state.event_queue;
    let stats = &mut state.stats_hier;
    let config = &state.config;
    let mut ctx = HandleCtx { scheduler, stats, config, cycle, self_id };
    match id {
        id if id == CacheId::new(0) => state.core.l1_i_cache.handle(packet, source, &mut ctx),
        id if id == CacheId::new(1) => state.core.l1_d_cache.handle(packet, source, &mut ctx),
        id if id == CacheId::new(2) => state.core.l2_cache.handle(packet, source, &mut ctx),
        id if id == CacheId::new(3) => state.l3_cache.handle(packet, source, &mut ctx),
        _ => {}
    }
}

/// Suppress unused-import warnings until the `MemCtrlId` constant is used
/// for routing in a follow-up.
const _: fn() = || {
    let _ = MemCtrlId::new(0);
};
