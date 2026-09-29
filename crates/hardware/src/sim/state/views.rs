//! What a pipeline stage other than commit works on.
//!
//! [`StageCtx`] reads the hart, drives the core's private
//! micro-architecture (TLBs, predictor, caches), counts stats and
//! schedules packets. It cannot write a register, a CSR, the PC or
//! memory: those belong to commit, which works on the full
//! [`CoreCtx`](super::CoreCtx). The one memory write it makes is the
//! page-table walker's A-bit update, which the spec lets happen
//! speculatively.

use std::ops::Deref;

use super::memory::TranslateResult;
use super::write_log::Writer;
use super::{SharedState, csr, memory};
use crate::common::{AccessType, CsrAddr, PteUpdate, VirtAddr};
use crate::core::exec::arch::ArchState;
use crate::core::units::mmu::ptw::WalkState;
use crate::core::{CoreUnits, Hart};
use crate::isa::op::MemWidth;
use crate::sim::events::EventQueue;
use crate::sim::stats::Counter;
use crate::sim::stats::paths::HartPaths;

/// A stage's view of its core: the hart read-only, the micro-architecture
/// mutable, and the uncore's stats and event queue.
///
/// Architectural state is out of reach; this does not compile:
///
/// ```compile_fail
/// fn execute(state: &mut rvsim_core::StageCtx<'_>) {
///     state.hart().regs.write(rvsim_core::RegIdx::new(1), 0);
/// }
/// ```
///
/// and neither does a CSR or memory write:
///
/// ```compile_fail
/// fn execute(state: &mut rvsim_core::StageCtx<'_>) {
///     state.csr_write(rvsim_core::CsrAddr::new(0x300), 0);
/// }
/// ```
#[derive(Debug)]
pub struct StageCtx<'a> {
    hart: &'a Hart,
    core: &'a mut CoreUnits,
    shared: &'a mut SharedState,
}

/// What a hardware A/D update found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PteUpdateOutcome {
    /// The bits were set and this PTE written.
    Written(u64),
    /// The bits were already set.
    AlreadySet,
    /// The PTE has changed since the walk; the access must translate again.
    Changed,
}

impl ArchState for StageCtx<'_> {
    fn hart(&self) -> &Hart {
        self.hart
    }

    fn csr_read(&self, addr: CsrAddr) -> u64 {
        csr::read(self.hart, self.shared, addr)
    }

    fn csr_read_for_update(&self, addr: CsrAddr) -> u64 {
        csr::read_for_update(self.hart, self.shared, addr)
    }

    fn tracing(&self) -> bool {
        self.shared.config.general.trace_instructions
    }
}

impl Deref for StageCtx<'_> {
    type Target = SharedState;

    fn deref(&self) -> &SharedState {
        self.shared
    }
}

impl<'a> StageCtx<'a> {
    pub(super) const fn new(
        hart: &'a Hart,
        core: &'a mut CoreUnits,
        shared: &'a mut SharedState,
    ) -> Self {
        Self { hart, core, shared }
    }

    /// The hart's architectural state.
    #[inline]
    #[must_use]
    pub const fn hart(&self) -> &Hart {
        self.hart
    }

    /// The core's private micro-architecture.
    #[inline]
    #[must_use]
    pub const fn core(&self) -> &CoreUnits {
        self.core
    }

    /// The core's private micro-architecture, to drive it.
    #[inline]
    pub const fn core_mut(&mut self) -> &mut CoreUnits {
        self.core
    }

    /// The stat counter `stat`.
    #[inline]
    pub fn counter(&mut self, stat: crate::sim::stats::StatId) -> &mut Counter {
        self.shared.stats.counter(stat)
    }

    /// The event queue, to schedule a packet.
    #[inline]
    pub const fn events(&mut self) -> &mut EventQueue {
        &mut self.shared.event_queue
    }

    /// Stat paths of the hart this view executes.
    #[inline]
    #[must_use]
    pub fn hart_paths(&self) -> HartPaths {
        self.shared.hart_stat_paths[self.hart.hart_id.as_index()]
    }

    /// Begins (or completes) translation of a virtual address.
    pub fn translate(&mut self, vaddr: VirtAddr, access: AccessType, size: u64) -> TranslateResult {
        memory::translate(self.core, self.hart, self.shared, vaddr, access, size)
    }

    /// Resumes a walk that was parked waiting on a PTE response.
    pub fn translate_continue(
        &mut self,
        state: WalkState,
        raw_pte: u64,
        bus_transit_cycles: u64,
    ) -> TranslateResult {
        memory::translate_continue(
            self.core,
            self.hart,
            self.shared,
            state,
            raw_pte,
            bus_transit_cycles,
        )
    }

    /// Sets a leaf PTE's A/D bits as the hardware does, atomically with
    /// the check that the PTE still holds the value its walk found.
    pub fn apply_pte_update(&mut self, update: &PteUpdate) -> PteUpdateOutcome {
        let Some(current) = self.shared.memory.read(update.pte_addr, 8) else {
            return PteUpdateOutcome::Changed;
        };
        match update.applied_to(current) {
            None => PteUpdateOutcome::Changed,
            Some(pte) if pte == current => PteUpdateOutcome::AlreadySet,
            Some(pte) => {
                let writer = Writer::Hart(self.hart.hart_id);
                self.shared.publish_write(writer, update.pte_addr, pte, MemWidth::Double);
                PteUpdateOutcome::Written(pte)
            }
        }
    }

    /// Reads a CSR.
    #[must_use]
    pub fn csr_read(&self, addr: CsrAddr) -> u64 {
        csr::read(self.hart, self.shared, addr)
    }

    /// The value a CSR read-modify-write starts from.
    #[must_use]
    pub fn csr_read_for_update(&self, addr: CsrAddr) -> u64 {
        csr::read_for_update(self.hart, self.shared, addr)
    }

    /// True when the hart implements the CSR at `addr`.
    #[must_use]
    pub const fn is_valid_csr(&self, addr: CsrAddr) -> bool {
        self.hart.is_valid_csr(addr)
    }

    /// True when an execute trigger fires for `pc` at the current privilege.
    #[must_use]
    pub fn check_execute_trigger(&self, pc: u64) -> bool {
        self.hart.check_execute_trigger(pc)
    }

    /// True when a load trigger fires for `addr` at the current privilege.
    #[must_use]
    pub fn check_load_trigger(&self, addr: u64) -> bool {
        self.hart.check_load_trigger(addr)
    }

    /// True when a store trigger fires for `addr` at the current privilege.
    #[must_use]
    pub fn check_store_trigger(&self, addr: u64) -> bool {
        self.hart.check_store_trigger(addr)
    }
}
