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

use crate::arch::Hart;
use crate::common::{AccessType, PhysAddr, SimError, VirtAddr};
use crate::config::Config;
use crate::isa::csr::CsrAddr;
use crate::isa::privileged::{PrivilegeMode, Trap};
use crate::sim::components::{CacheId, ComponentId, MemCtrlId};
use crate::sim::events::Event;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::memory::write_log::Writer;
use crate::sim::packet::Packet;
use crate::sim::stats::Stats;
use crate::soc::devices::Uart;
use crate::soc::topology::{CacheSlot, PrivateCache};
use crate::system::snapshot::PipelineSnapshot;
use crate::system::state::SystemState;
use crate::system::{StatsDump, StatsEpoch, TraceControl, coherence_audit, loader};
use crate::uarch::mmu::TranslateOutcome;
use crate::uarch::pipeline::engine::PipelineDispatch;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

/// Where [`Simulator::run_to`] stops, besides the simulation ending.
/// Counts are relative to where the run starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StopAt {
    /// After this many cycles.
    pub cycles: Option<u64>,
    /// Once this many more instructions have retired, over all harts.
    pub instructions: Option<u64>,
    /// When any hart's next instruction to retire is at one of these
    /// addresses.
    pub pcs: Vec<u64>,
    /// When guest software asks to stop (the sim-control break command).
    pub guest_breaks: bool,
    /// When a captured console holds output the host has not taken.
    pub console_output: bool,
}

/// Why [`Simulator::run_to`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// The simulation ended with this exit code.
    Exited(u64),
    /// The cycle count was reached.
    Cycles,
    /// The instruction count was reached.
    Instructions,
    /// This hart reached the address.
    Pc {
        /// The hart.
        hart: usize,
    },
    /// Guest software asked to stop, with this label.
    GuestBreak {
        /// The guest's label.
        label: u64,
    },
    /// A captured console holds output the host has not taken.
    ConsoleOutput,
    /// The caller's `keep_going` said to stop.
    Cancelled,
}

/// Cycles between [`Simulator::run_to_with`]'s checks of `keep_going`.
const CANCEL_POLL_CYCLES: u64 = 1 << 16;

/// The most cycles one quiet skip covers, so a system idle forever still
/// polls `keep_going`.
const MAX_QUIET_SKIP: u64 = 1 << 32;

/// Top-level simulator: the system state and the order it ticks in.
#[derive(Debug)]
pub struct Simulator {
    /// The whole system: harts, cores and their pipelines, and the uncore.
    pub(crate) state: SystemState,
    /// Privilege mode of each hart at the start of the current tick, kept
    /// between ticks to avoid reallocating.
    prev_privileges: Vec<PrivilegeMode>,
    /// Count an idle core's cycle instead of ticking its pipeline, and skip
    /// the cycles in which the whole system only waits. The result is the
    /// same; tests turn it off to check that.
    skip_idle_cores: bool,
}

impl Simulator {
    /// Wraps an existing `SystemState`, pointing each core's fetch at its
    /// hart's PC. Use this when the caller needs to interleave setup between
    /// state construction and the first tick (e.g. loading an ELF image and
    /// registering HTIF, which set the reset PC).
    pub(crate) fn new(mut state: SystemState) -> Self {
        for core in 0..state.cores.len() {
            let (pipeline, ctx) = state.pipeline_ctx(core);
            pipeline.restart_fetch_at(ctx.hart.pc);
        }
        let prev_privileges = state.harts.iter().map(|h| h.privilege).collect();
        Self { state, prev_privileges, skip_idle_cores: true }
    }

    /// Convenience constructor: builds the exit-signal `Arc`, the `SystemState`,
    /// and the `Simulator` together. Use this when the caller doesn't need
    /// to touch the state between construction and pipeline start.
    pub fn build(config: &Config, disk_path: &str) -> Self {
        let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
        Self::new(SystemState::new(config, disk_path, exit_signal))
    }

    /// Loads the ELF image `data` as the program to run: its segments go
    /// into RAM, hart 0 starts at its entry point in machine mode, and an
    /// HTIF device is placed at its `tohost` symbol when it has one.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::NotAnElf`] when `data` is not an ELF image.
    pub fn load_elf(&mut self, data: &[u8]) -> Result<(), SimError> {
        let loaded =
            loader::try_load_elf(data, &mut self.state.memory).ok_or(SimError::NotAnElf)?;
        self.set_pc(0, loaded.entry);
        if let Some(tohost) = loaded.tohost_addr {
            let exit_signal = Arc::clone(&self.state.exit_signal);
            self.state.uncore.add_htif(tohost, &exit_signal);
            self.state.direct_mode = false;
            self.state.harts[0].privilege = PrivilegeMode::Machine;
        }
        self.sync_arch_regs();
        Ok(())
    }

    /// Places firmware, a kernel and a device tree in RAM as `boot` says
    /// and points every hart at the firmware.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::FileRead`] when an image cannot be read.
    pub fn boot_kernel(&mut self, boot: &loader::KernelBoot) -> Result<(), SimError> {
        let config = self.state.config.clone();
        loader::setup_kernel_load(&mut self.state, &config, boot)?;
        self.state.direct_mode = false;
        self.sync_arch_regs();
        Ok(())
    }

    /// The number of harts.
    #[must_use]
    pub const fn hart_count(&self) -> usize {
        self.state.harts.len()
    }

    /// Hart `hart`'s architectural state.
    ///
    /// # Panics
    ///
    /// Panics if `hart` is not a hart index.
    #[must_use]
    pub fn hart(&self, hart: usize) -> &Hart {
        &self.state.harts[hart]
    }

    /// Hart `hart`'s architectural state, to change it. Use
    /// [`Self::set_pc`] for the PC, which the pipeline must hear about.
    ///
    /// # Panics
    ///
    /// Panics if `hart` is not a hart index.
    pub fn hart_mut(&mut self, hart: usize) -> &mut Hart {
        &mut self.state.harts[hart]
    }

    /// Reads CSR `addr` on `hart` the way a CSR instruction would; `None`
    /// when the hart does not implement it.
    ///
    /// # Panics
    ///
    /// Panics if `hart` is not a hart index.
    pub fn read_csr(&mut self, hart: usize, addr: CsrAddr) -> Option<u64> {
        let hart_id = self.state.harts[hart].hart_id;
        let core = self.state.topology.core_of_hart(hart_id)?;
        let ctx = self.state.core_ctx(core.as_index());
        ctx.is_valid_csr(addr).then(|| ctx.csr_read(addr))
    }

    /// Translates `vaddr` for a read on `hart` through its current page
    /// tables, walking them now rather than through the pipeline.
    ///
    /// # Errors
    ///
    /// Returns the trap the access would take.
    ///
    /// # Panics
    ///
    /// Panics if `hart` is not a hart index.
    pub fn translate_now(&mut self, hart: usize, vaddr: VirtAddr) -> Result<PhysAddr, Trap> {
        let hart_id = self.state.harts[hart].hart_id;
        let Some(core) = self.state.topology.core_of_hart(hart_id) else {
            return Ok(PhysAddr::new(vaddr.val()));
        };
        let core = core.as_index();
        let mut outcome = self.state.core_ctx(core).translate(vaddr, AccessType::Read, 8);
        loop {
            match outcome {
                TranslateOutcome::Ready(result) => {
                    return result.trap.map_or(Ok(result.paddr), Err);
                }
                TranslateOutcome::NeedPte { pte_addr, state } => {
                    let raw_pte = self.probe_mem_load(pte_addr, 8);
                    outcome = self.state.core_ctx(core).translate_continue(state, raw_pte, 0);
                }
            }
        }
    }

    /// Cycles since the system started, carried across checkpoints.
    #[must_use]
    pub const fn cycle(&self) -> u64 {
        self.state.uncore.cycle
    }

    /// Instructions retired by every hart since the system started.
    #[must_use]
    pub fn instructions_retired(&self) -> u64 {
        self.state.instructions_retired()
    }

    /// The stats tree.
    #[must_use]
    pub const fn stats(&self) -> &Stats {
        &self.state.uncore.stats
    }

    /// Cycles and instructions retired since the stats were last reset.
    #[must_use]
    pub fn stats_window(&self) -> (u64, u64) {
        self.state.stats_window()
    }

    /// Where the current stats window began.
    #[must_use]
    pub const fn stats_epoch(&self) -> StatsEpoch {
        self.state.uncore.stats_epoch
    }

    /// Zeroes every stat and starts a new window here.
    pub fn reset_stats(&mut self) {
        self.state.reset_stats();
    }

    /// The stats the guest dumped through the sim-control device, oldest
    /// first.
    #[must_use]
    pub fn stats_dumps(&self) -> &[StatsDump] {
        &self.state.uncore.stats_dumps
    }

    /// What the trace macros print.
    #[must_use]
    pub const fn trace(&self) -> &TraceControl {
        &self.state.uncore.trace
    }

    /// What the trace macros print, to change it.
    pub const fn trace_mut(&mut self) -> &mut TraceControl {
        &mut self.state.uncore.trace
    }

    /// Whether the system runs in direct mode: no translation, a flat
    /// memory, and `ecall` ending the program.
    #[must_use]
    pub const fn direct_mode(&self) -> bool {
        self.state.uncore.direct_mode
    }

    /// Sets direct mode; see [`Self::direct_mode`].
    pub const fn set_direct_mode(&mut self, direct: bool) {
        self.state.uncore.direct_mode = direct;
    }

    /// The last instructions `hart` retired, oldest first, as `(pc,
    /// encoding)` pairs.
    ///
    /// # Panics
    ///
    /// Panics if `hart` is not a hart index.
    #[must_use]
    pub fn pc_trace(&self, hart: usize) -> &[(u64, u32)] {
        &self.state.uncore.per_hart_debug[hart].pc_trace
    }

    /// Whether idle cores have their cycles counted instead of ticked and
    /// cycles in which the whole system only waits are skipped. On by
    /// default; the result is the same either way.
    #[must_use]
    pub const fn skip_idle_cores(&self) -> bool {
        self.skip_idle_cores
    }

    /// Sets whether idle time is skipped; see [`Self::skip_idle_cores`].
    pub const fn set_skip_idle_cores(&mut self, skip: bool) {
        self.skip_idle_cores = skip;
    }

    /// Opens `path` as the commit log every retired instruction is written
    /// to.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::FileRead`] when the file cannot be created.
    #[cfg(feature = "commit-log")]
    pub fn open_commit_log(&mut self, path: &str) -> Result<(), SimError> {
        self.state.uncore.open_commit_log(path)
    }

    /// The `len` bytes of RAM at `paddr`; `None` outside RAM.
    #[must_use]
    pub fn read_phys_bytes(&self, paddr: PhysAddr, len: usize) -> Option<Box<[u8]>> {
        self.state.uncore.memory.read_bytes(paddr, len)
    }

    /// What the guest has printed to a captured console since the last
    /// call; empty when the console is not captured.
    pub fn take_console_output(&mut self) -> Vec<u8> {
        self.state.uncore.bus.uart_mut().map(Uart::take_output).unwrap_or_default()
    }

    /// Types `bytes` into the console.
    pub fn send_console_input(&mut self, bytes: &[u8]) {
        if let Some(uart) = self.state.uncore.bus.uart_mut() {
            uart.send_input(bytes);
        }
    }

    /// What core `core`'s pipeline holds at the end of the last tick.
    ///
    /// # Panics
    ///
    /// Panics if `core` is not a core index.
    #[must_use]
    pub fn pipeline_snapshot(&self, core: usize) -> PipelineSnapshot {
        let width = self.state.uncore.config.pipeline.width;
        PipelineSnapshot::from(&self.state.cores[core].pipeline.snapshot(width))
    }

    /// Every coherence invariant the private caches and the home agent
    /// break right now; empty when they agree.
    #[must_use]
    pub fn audit_coherence(&self) -> Vec<coherence_audit::Violation> {
        coherence_audit::audit(&self.state)
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
        let uncore = &mut self.state.uncore;
        uncore.bus.drain_devices(&mut uncore.memory);
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

    /// Runs at least one cycle, then until one of `stop`'s conditions
    /// holds or the simulation ends, checking after every cycle, and says
    /// which. Running on from a stop at a PC therefore moves past it.
    ///
    /// # Errors
    ///
    /// Returns the [`SimError`] a tick raised.
    pub fn run_to(&mut self, stop: &StopAt) -> Result<StopReason, SimError> {
        self.run_to_with(stop, || true)
    }

    /// [`Self::run_to`], asking `keep_going` every so often whether to go
    /// on (a host checking for Ctrl-C).
    ///
    /// # Errors
    ///
    /// Returns the [`SimError`] a tick raised.
    pub fn run_to_with(
        &mut self,
        stop: &StopAt,
        mut keep_going: impl FnMut() -> bool,
    ) -> Result<StopReason, SimError> {
        if let Some(code) = self.take_exit() {
            return Ok(StopReason::Exited(code));
        }
        let start_cycle = self.state.cycle;
        let start_instructions = self.state.instructions_retired();
        loop {
            let before = self.state.cycle;
            let limit = stop.cycles.map_or(u64::MAX, |n| start_cycle + n - before);
            self.advance(stop, limit)?;
            if let Some(reason) = self.stop_reason(stop, start_cycle, start_instructions) {
                return Ok(reason);
            }
            let polled = before / CANCEL_POLL_CYCLES != self.state.cycle / CANCEL_POLL_CYCLES;
            if polled && !keep_going() {
                return Ok(StopReason::Cancelled);
            }
        }
    }

    /// Ticks one cycle, then skips the cycles after it in which nothing
    /// but time would pass, `limit` cycles in all at most.
    fn advance(&mut self, stop: &StopAt, limit: u64) -> Result<(), SimError> {
        self.tick()?;
        let quiet = self.quiet_cycles(stop).min(limit.saturating_sub(1)).min(MAX_QUIET_SKIP);
        if quiet > 0 {
            self.skip_quiet_cycles(quiet);
        }
        Ok(())
    }

    /// How many cycles from here would change nothing but time: every core
    /// idle in WFI, nothing in flight, and no device, timer comparator or
    /// memory controller due to act. Stops `stop` would take after one more
    /// cycle end the skip too, so a run stops where ticking would stop it.
    fn quiet_cycles(&mut self, stop: &StopAt) -> u64 {
        let state = &mut self.state;
        let now = state.cycle;
        let settled = self.skip_idle_cores
            && !state.trace.armed
            && !state.config.general.trace_instructions
            && state.check_exit().is_none()
            && state.panic_detected_at_cycle.is_none()
            && state.event_queue.is_empty()
            && state.coherence.as_ref().is_none_or(|fabric| fabric.is_quiet(now + 1))
            && !(stop.guest_breaks && state.uncore.pending_break.is_some())
            && !(stop.console_output && state.bus.console_has_output())
            && !state.harts.iter().any(|hart| stop.pcs.contains(&hart.pc));
        if !settled || !(0..self.core_count()).all(|core| self.core_is_idle(core)) {
            return 0;
        }
        let state = &self.state;
        let timers = state.harts.iter().filter_map(|hart| {
            let sstc = hart.csrs.menvcfg & crate::isa::csr::MENVCFG_STCE != 0;
            sstc.then(|| state.bus.ticks_until_mtime(hart.csrs.stimecmp)).flatten()
        });
        let memory = state.mem_controller.quiet_until(now + 1).map(|at| at - (now + 1));
        let devices = state.bus.quiet_ticks();
        let stimecmp = timers.min().map(|ticks| ticks - 1);
        [devices, stimecmp, memory].into_iter().flatten().min().unwrap_or(u64::MAX)
    }

    /// Advances `cycles` quiet cycles at once, leaving the system exactly as
    /// ticking through them would.
    fn skip_quiet_cycles(&mut self, cycles: u64) {
        self.state.bus.skip_ticks(cycles);
        for core in 0..self.core_count() {
            self.state.core_ctx(core).skip_quiet_cycles(cycles);
        }
        self.state.cycle += cycles;
        let uncore = &mut self.state.uncore;
        let mut ctx = HandleCtx {
            scheduler: &mut uncore.event_queue,
            stats: &mut uncore.stats,
            memory: &mut uncore.memory,
            config: &uncore.config,
            cycle: uncore.cycle,
            self_id: ComponentId::MemCtrl(MemCtrlId::new(0)),
        };
        uncore.mem_controller.skip_quiet(&mut ctx);
    }

    fn stop_reason(
        &mut self,
        stop: &StopAt,
        start_cycle: u64,
        start_instructions: u64,
    ) -> Option<StopReason> {
        if let Some(code) = self.take_exit() {
            return Some(StopReason::Exited(code));
        }
        if stop.guest_breaks
            && let Some(label) = self.state.uncore.pending_break.take()
        {
            return Some(StopReason::GuestBreak { label });
        }
        if stop.console_output && self.state.bus.console_has_output() {
            return Some(StopReason::ConsoleOutput);
        }
        if !stop.pcs.is_empty()
            && let Some(hart) = self.state.harts.iter().position(|hart| stop.pcs.contains(&hart.pc))
        {
            return Some(StopReason::Pc { hart });
        }
        if stop
            .instructions
            .is_some_and(|n| self.state.instructions_retired() - start_instructions >= n)
        {
            return Some(StopReason::Instructions);
        }
        if stop.cycles.is_some_and(|n| self.state.cycle - start_cycle >= n) {
            return Some(StopReason::Cycles);
        }
        None
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
                if self.skip_idle_cores && self.core_is_idle(core) {
                    self.state.core_ctx(core).count_idle_cycle();
                    continue;
                }
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

    fn core_is_idle(&self, core: usize) -> bool {
        let hart = self.state.topology.cores[core].hart_ids[0];
        let units = &self.state.cores[core];
        units.pipeline.is_idle(&self.state.harts[hart.as_index()], &units.units)
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
        let uncore = &mut self.state.uncore;
        let mut ctx = HandleCtx {
            scheduler: &mut uncore.event_queue,
            stats: &mut uncore.stats,
            memory: &mut uncore.memory,
            config: &uncore.config,
            cycle: uncore.cycle,
            self_id: ComponentId::MemCtrl(MemCtrlId::new(0)),
        };
        uncore.mem_controller.tick(&mut ctx);
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
                let uncore = &mut self.state.uncore;
                let mut ctx = HandleCtx {
                    scheduler: &mut uncore.event_queue,
                    stats: &mut uncore.stats,
                    memory: &mut uncore.memory,
                    config: &uncore.config,
                    cycle: uncore.cycle,
                    self_id: ComponentId::Bus,
                };
                uncore.bus.handle(packet, source, &mut ctx);
            }
            ComponentId::MemCtrl(id) => {
                let uncore = &mut self.state.uncore;
                let mut ctx = HandleCtx {
                    scheduler: &mut uncore.event_queue,
                    stats: &mut uncore.stats,
                    memory: &mut uncore.memory,
                    config: &uncore.config,
                    cycle: uncore.cycle,
                    self_id: ComponentId::MemCtrl(id),
                };
                uncore.mem_controller.handle(packet, source, &mut ctx);
            }
            ComponentId::Fabric => {
                let uncore = &mut self.state.uncore;
                if let Some(fabric) = uncore.coherence.as_mut() {
                    let mut ctx = HandleCtx {
                        scheduler: &mut uncore.event_queue,
                        stats: &mut uncore.stats,
                        memory: &mut uncore.memory,
                        config: &uncore.config,
                        cycle: uncore.cycle,
                        self_id: ComponentId::Fabric,
                    };
                    fabric.handle(packet, source, &mut ctx);
                }
            }
            ComponentId::Device(id) => {
                let uncore = &mut self.state.uncore;
                let mut ctx = HandleCtx {
                    scheduler: &mut uncore.event_queue,
                    stats: &mut uncore.stats,
                    memory: &mut uncore.memory,
                    config: &uncore.config,
                    cycle: uncore.cycle,
                    self_id: ComponentId::Device(id),
                };
                uncore.bus.handle_device(id, packet, source, &mut ctx);
            }
        }
    }

    /// Advances the coherence fabric one cycle: moves messages through the
    /// interconnect and lets the home agent act on what arrived.
    fn tick_fabric(&mut self) {
        let uncore = &mut self.state.uncore;
        let Some(fabric) = uncore.coherence.as_mut() else { return };
        let mut ctx = HandleCtx {
            scheduler: &mut uncore.event_queue,
            stats: &mut uncore.stats,
            memory: &mut uncore.memory,
            config: &uncore.config,
            cycle: uncore.cycle,
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
    /// inspect memory without driving the full pipeline. RAM addresses read
    /// the memory image; MMIO addresses dispatch a `MemReq` through the
    /// bus's `Handle` impl with a local event queue and read the response
    /// data out of the synchronously-scheduled `MemResp`.
    ///
    /// Not for use inside pipeline stages — those emit `MemReq` packets
    /// through the global event queue and consume responses via the
    /// mailbox-drain stage.
    pub fn probe_mem_load(&mut self, paddr: crate::common::PhysAddr, width: u8) -> u64 {
        if let Some(value) = self.state.memory.read(paddr, usize::from(width)) {
            return value;
        }
        self.probe_mmio(paddr, width, crate::sim::packet::MemOp::Read)
    }

    /// Synchronously writes `width` bytes to physical memory. RAM takes the
    /// write as an external one every hart observes; for MMIO a `MemReq` is
    /// dispatched through the bus's `Handle` impl so the device's side
    /// effect runs.
    pub fn probe_mem_store(&mut self, paddr: crate::common::PhysAddr, value: u64, width: u8) {
        if self.state.memory.ram().is_some_and(|ram| ram.contains(paddr, u64::from(width))) {
            self.state.memory.write(Writer::External, paddr, value, usize::from(width));
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
        let uncore = &mut self.state.uncore;
        let mut ctx = HandleCtx {
            scheduler: &mut local_queue,
            stats: &mut local_stats,
            memory: &mut uncore.memory,
            config: &uncore.config,
            cycle: uncore.cycle,
            self_id: ComponentId::Bus,
        };
        let _ = uncore.bus.probe_device(
            Packet::MemReq { req_id, paddr, vaddr: None, pc: None, size: access_size, op },
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
fn dispatch_to_cache(state: &mut SystemState, id: CacheId, packet: Packet, source: ComponentId) {
    let self_id = ComponentId::Cache(id);
    let Some(slot) = state.topology.locate_cache(id) else { return };
    // Split-borrow: the HandleCtx borrows the uncore's event queue, stats
    // and config while the cache itself comes from a core or from the
    // uncore's LLC field.
    let SystemState { cores, uncore, .. } = state;
    let cycle = uncore.cycle;
    let mut ctx = HandleCtx {
        scheduler: &mut uncore.event_queue,
        stats: &mut uncore.stats,
        memory: &mut uncore.memory,
        config: &uncore.config,
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
        CacheSlot::Llc => uncore.l3_cache.handle(packet, source, &mut ctx),
    }
}
