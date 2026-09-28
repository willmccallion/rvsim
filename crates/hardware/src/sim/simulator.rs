//! Simulator: owns the system state, whose cores carry their pipelines, and
//! drives the global event queue.
//!
//! Each `tick()` runs the fixed order described in
//! `docs/architecture/multicore.md`:
//! 1. Uncore pre-cycle: exit / panic checks and one tick of every device.
//! 2. Per-hart pre-tick (interrupt lines into `mip`, hang detection), then
//!    the clock advances and mode cycles are charged.
//! 3. Drain events scheduled for the new cycle into their targets.
//! 4. Tick every pipeline in core order.
//! 5. Drain again so packets the pipelines just emitted reach their targets.
//! 6. Tick memory controllers, then the coherence fabric, then drain once more.
//! 7. Per-hart post-tick.
//!
//! Memory traffic (instruction fetch, load, store, page-table walk) flows
//! exclusively through scheduled `MemReq` / `MemResp` packets.

use crate::common::SimError;
use crate::config::Config;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::pipeline::engine::PipelineDispatch;
use crate::sim::components::{CacheId, ComponentId, MemCtrlId};
use crate::sim::events::Event;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::Packet;
use crate::sim::state::SimState;
use crate::sim::topology::{CacheSlot, PrivateCache};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

/// Top-level simulator: the system state and the order it ticks in.
#[derive(Debug)]
pub struct Simulator {
    /// The whole system: harts, cores and their pipelines, and the uncore.
    pub state: SimState,
    /// Privilege mode of each hart at the start of the current tick, kept
    /// between ticks to avoid reallocating.
    prev_privileges: Vec<PrivilegeMode>,
}

unsafe impl Send for Simulator {}
unsafe impl Sync for Simulator {}

impl Simulator {
    /// Wraps an existing `SimState`, pointing each core's fetch at its
    /// hart's PC. Use this when the caller needs to interleave setup between
    /// state construction and the first tick (e.g. loading an ELF image and
    /// registering HTIF, which set the reset PC).
    pub fn new(mut state: SimState) -> Self {
        for core in 0..state.cores.len() {
            let (pipeline, ctx) = state.pipeline_ctx(core);
            pipeline.restart_fetch_at(ctx.hart.pc);
        }
        let prev_privileges = state.harts.iter().map(|h| h.privilege).collect();
        Self { state, prev_privileges }
    }

    /// Convenience constructor: builds the exit-signal `Arc`, the `SimState`,
    /// and the `Simulator` together. Use this when the caller doesn't need
    /// to touch the state between construction and pipeline start.
    pub fn build(config: &Config, disk_path: &str) -> Self {
        let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
        Self::new(SimState::new(config, disk_path, exit_signal))
    }

    /// Discards every core's speculative work, leaves each hart at its
    /// committed PC, and runs the memory system until every committed store
    /// has taken effect: the self-contained architectural state a
    /// checkpoint records. Like gem5's drain, it takes simulated time and
    /// perturbs the timing of a run that continues afterwards.
    pub fn drain(&mut self) {
        for core in 0..self.core_count() {
            let (pipeline, mut ctx) = self.state.pipeline_ctx(core);
            pipeline.flush(&mut ctx);
        }
        while self.drain_writes_for_a_cycle() {}
        let shared = &mut self.state.shared;
        shared.bus.drain_devices();
        for (paddr, len) in shared.bus.take_dma_writes() {
            shared.memory.record_external_write_range(paddr, len);
        }
    }

    /// Runs one cycle of the memory system in which each drained core sends
    /// its next committed write. Returns whether any core still has one
    /// outstanding.
    fn drain_writes_for_a_cycle(&mut self) -> bool {
        self.state.advance_cycle();
        self.drain_events();
        let mut pending = false;
        for core in 0..self.core_count() {
            let (pipeline, mut ctx) = self.state.pipeline_ctx(core);
            pending |= pipeline.drain_writes(&mut ctx);
        }
        self.drain_events();
        self.tick_mem_controller();
        self.tick_fabric();
        self.drain_events();
        pending
    }

    /// Number of cores.
    #[must_use]
    pub const fn core_count(&self) -> usize {
        self.state.cores.len()
    }

    /// Synchronize every hart's architectural state into its pipeline:
    /// the register file into the O3 PRF, and the PC into fetch.
    ///
    /// Must be called after all register and PC initialization (loader
    /// setup, etc.) but before the first pipeline tick.
    pub fn sync_arch_regs(&mut self) {
        for core in 0..self.core_count() {
            let (pipeline, ctx) = self.state.pipeline_ctx(core);
            pipeline.restart_fetch_at(ctx.hart.pc);
            if let PipelineDispatch::OutOfOrder(p) = pipeline {
                p.engine.sync_arch_regs(&ctx);
            }
        }
    }

    /// Points `hart` at `pc`: its architectural PC, and its fetch PC after
    /// everything its pipeline had in flight is dropped.
    pub fn set_pc(&mut self, hart: usize, pc: u64) {
        self.state.harts[hart].pc = pc;
        let hart_id = self.state.harts[hart].hart_id;
        let Some(core) = self.state.topology.core_of_hart(hart_id) else { return };
        let (pipeline, mut ctx) = self.state.pipeline_ctx(core.as_index());
        pipeline.flush(&mut ctx);
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
        for (slot, hart) in self.prev_privileges.iter_mut().zip(&self.state.harts) {
            *slot = hart.privilege;
        }
        let run_cycle = self.state.pre_cycle()?;
        if run_cycle {
            for core in 0..self.core_count() {
                let hart = self.state.topology.cores[core].hart_ids[0];
                let irqs = self.state.bus.hart_irqs(hart);
                self.state.core_ctx(core).pre_tick(irqs);
            }
            self.state.advance_cycle();
            for core in 0..self.core_count() {
                self.state.core_ctx(core).track_mode_cycles();
            }
        }
        // First drain: deliver events scheduled for cycles <= now into
        // their targets (filling pipeline mailboxes with responses from
        // previous cycles' emissions).
        self.drain_events();
        if run_cycle {
            for core in 0..self.core_count() {
                self.scoped_to_hart(core, |sim| {
                    let (pipeline, mut ctx) = sim.state.pipeline_ctx(core);
                    pipeline.tick(&mut ctx);
                });
            }
            self.leave_hart_scope();
        }
        // Second drain: events the pipelines just scheduled (MemReqs to L1)
        // reach their target component handlers this cycle so the next
        // cycle's start-of-tick drain delivers their responses.
        self.drain_events();
        self.tick_mem_controller();
        self.tick_fabric();
        // Drain again so commands / responses emitted during the memory
        // controller's and fabric's ticks reach their targets on the
        // following cycle.
        self.drain_events();
        for core in 0..self.core_count() {
            let prev = self.prev_privileges[self.state.topology.cores[core].hart_ids[0].as_index()];
            self.scoped_to_hart(core, |sim| sim.state.core_ctx(core).post_tick(prev));
        }
        self.leave_hart_scope();
        for op in self.state.bus.take_sim_ops() {
            self.state.apply_sim_op(op);
        }
        Ok(())
    }

    /// Runs `f` inside `core`'s hart span with the trace armed for that
    /// hart, so every event it emits is tagged and filtered per hart.
    fn scoped_to_hart(&mut self, core: usize, f: impl FnOnce(&mut Self)) {
        let hart = self.state.topology.cores[core].hart_ids[0];
        let cycle = self.state.cycle;
        self.state.config.general.trace_instructions = self.state.trace.applies(Some(hart), cycle);
        let span = tracing::trace_span!(target: "rvsim::hart", "hart", id = hart.val(), cycle);
        let _entered = span.enter();
        f(self);
    }

    fn leave_hart_scope(&mut self) {
        let cycle = self.state.cycle;
        self.state.config.general.trace_instructions = self.state.trace.applies(None, cycle);
    }

    fn tick_mem_controller(&mut self) {
        let shared = &mut self.state.shared;
        let mut ctx = HandleCtx {
            scheduler: &mut shared.event_queue,
            stats: &mut shared.stats,
            memory: &mut shared.memory,
            config: &shared.config,
            cycle: shared.cycle,
            self_id: ComponentId::MemCtrl(MemCtrlId::new(0)),
        };
        shared.mem_controller.tick(&mut ctx);
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
            ComponentId::Pipeline(id) => {
                if let Some(core) = self.state.cores.get_mut(id.as_index()) {
                    core.pipeline.deliver(source, packet);
                }
            }
            ComponentId::Cache(id) => {
                dispatch_to_cache(&mut self.state, id, packet, source);
            }
            ComponentId::Bus => {
                let shared = &mut self.state.shared;
                let mut ctx = HandleCtx {
                    scheduler: &mut shared.event_queue,
                    stats: &mut shared.stats,
                    memory: &mut shared.memory,
                    config: &shared.config,
                    cycle: shared.cycle,
                    self_id: ComponentId::Bus,
                };
                shared.bus.handle(packet, source, &mut ctx);
                for (paddr, len) in shared.bus.take_dma_writes() {
                    shared.memory.record_external_write_range(paddr, len);
                }
            }
            ComponentId::MemCtrl(id) => {
                let shared = &mut self.state.shared;
                let mut ctx = HandleCtx {
                    scheduler: &mut shared.event_queue,
                    stats: &mut shared.stats,
                    memory: &mut shared.memory,
                    config: &shared.config,
                    cycle: shared.cycle,
                    self_id: ComponentId::MemCtrl(id),
                };
                shared.mem_controller.handle(packet, source, &mut ctx);
            }
            ComponentId::Fabric => {
                let shared = &mut self.state.shared;
                if let Some(fabric) = shared.coherence.as_mut() {
                    let mut ctx = HandleCtx {
                        scheduler: &mut shared.event_queue,
                        stats: &mut shared.stats,
                        memory: &mut shared.memory,
                        config: &shared.config,
                        cycle: shared.cycle,
                        self_id: ComponentId::Fabric,
                    };
                    fabric.handle(packet, source, &mut ctx);
                }
            }
            ComponentId::Device(id) => {
                let shared = &mut self.state.shared;
                let mut ctx = HandleCtx {
                    scheduler: &mut shared.event_queue,
                    stats: &mut shared.stats,
                    memory: &mut shared.memory,
                    config: &shared.config,
                    cycle: shared.cycle,
                    self_id: ComponentId::Device(id),
                };
                shared.bus.handle_device(id, packet, source, &mut ctx);
                for (paddr, len) in shared.bus.take_dma_writes() {
                    shared.memory.record_external_write_range(paddr, len);
                }
            }
            ComponentId::Hart(_) | ComponentId::Core(_) => {
                // Reserved for future per-hart packets.
            }
        }
    }

    /// Advances the coherence fabric one cycle: moves messages through the
    /// interconnect and lets the home agent act on what arrived.
    fn tick_fabric(&mut self) {
        let shared = &mut self.state.shared;
        let Some(fabric) = shared.coherence.as_mut() else { return };
        let mut ctx = HandleCtx {
            scheduler: &mut shared.event_queue,
            stats: &mut shared.stats,
            memory: &mut shared.memory,
            config: &shared.config,
            cycle: shared.cycle,
            self_id: ComponentId::Fabric,
        };
        fabric.tick(&mut ctx);
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
        if let Some(r) = self.state.bus.ram_region().filter(|r| r.contains(raw, u64::from(width))) {
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
    pub fn probe_mem_store(&mut self, paddr: crate::common::PhysAddr, value: u64, width: u8) {
        let raw = paddr.val();
        if let Some(r) = self.state.bus.ram_region().filter(|r| r.contains(raw, u64::from(width))) {
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
            self.state.memory.record_external_write(paddr);
            return;
        }
        let op = crate::sim::packet::MemOp::Write {
            data: crate::sim::packet::WriteData::Small(value),
            origin: crate::sim::packet::WriteOrigin::Host,
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
        let shared = &mut self.state.shared;
        let mut ctx = HandleCtx {
            scheduler: &mut local_queue,
            stats: &mut local_stats,
            memory: &mut shared.memory,
            config: &shared.config,
            cycle: shared.cycle,
            self_id: ComponentId::Bus,
        };
        let _ = shared.bus.probe_device(
            Packet::MemReq { req_id, paddr, vaddr: None, size: access_size, op },
            ComponentId::Pipeline(PipelineId::new(0)),
            &mut ctx,
        );
        while let Some(event) = local_queue.pop_ready(u64::MAX) {
            if let Packet::MemResp { req_id: rid, data, .. } = event.packet
                && rid == req_id
            {
                return match data {
                    MemRespData::Small(value) | MemRespData::Performed { value, .. } => value,
                    MemRespData::Line(_) | MemRespData::PerformedBytes { .. } => 0,
                };
            }
        }
        0
    }
}

/// Dispatches a packet to the cache identified by `id`.
fn dispatch_to_cache(state: &mut SimState, id: CacheId, packet: Packet, source: ComponentId) {
    let self_id = ComponentId::Cache(id);
    let Some(slot) = state.topology.locate_cache(id) else { return };
    // Split-borrow: the HandleCtx borrows the uncore's event queue, stats
    // and config while the cache itself comes from a core or from the
    // uncore's LLC field.
    let SimState { cores, shared, .. } = state;
    let cycle = shared.cycle;
    let mut ctx = HandleCtx {
        scheduler: &mut shared.event_queue,
        stats: &mut shared.stats,
        memory: &mut shared.memory,
        config: &shared.config,
        cycle,
        self_id,
    };
    match slot {
        CacheSlot::Private { core, which } => {
            let core = &mut cores[core.as_index()].units;
            let cache = match which {
                PrivateCache::L1I => &mut core.l1_i_cache,
                PrivateCache::L1D => &mut core.l1_d_cache,
                PrivateCache::L2 => &mut core.l2_cache,
            };
            cache.handle(packet, source, &mut ctx);
        }
        CacheSlot::Llc => shared.l3_cache.handle(packet, source, &mut ctx),
    }
}
