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

use super::{csr, memory};
use crate::arch::Hart;
use crate::arch::pmp::PmpResult;
use crate::arch::translation::PteUpdate;
use crate::common::{AccessType, PhysAddr, VirtAddr};
use crate::exec::state::ArchState;
use crate::isa::csr::CsrAddr;
use crate::isa::op::MemWidth;
use crate::isa::privileged::PrivilegeMode;
use crate::sim::events::EventQueue;
use crate::sim::memory::write_log::Writer;
use crate::sim::packet::CacheLevel;
use crate::sim::stats::Counter;
use crate::soc::uncore::Uncore;
use crate::uarch::CoreUnits;
use crate::uarch::mmu::TranslateOutcome;
use crate::uarch::mmu::ptw::WalkState;
use crate::uarch::prefetch::{LoadPrefetch, PagePlacer, PrefetchDrop};

/// A stage's view of its core: the hart read-only, the micro-architecture
/// mutable, and the uncore's stats and event queue.
///
/// Architectural state is out of reach: [`Self::hart`] hands out a shared
/// reference, and the view has no CSR or memory write. Only
/// [`CoreCtx`](super::CoreCtx), which commit holds, can change the hart.
#[derive(Debug)]
pub struct StageCtx<'a> {
    hart: &'a Hart,
    core: &'a mut CoreUnits,
    uncore: &'a mut Uncore,
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
        csr::read(self.hart, self.uncore, addr)
    }

    fn csr_read_for_update(&self, addr: CsrAddr) -> u64 {
        csr::read_for_update(self.hart, self.uncore, addr)
    }

    fn tracing(&self) -> bool {
        self.uncore.config.general.trace_instructions
    }
}

impl Deref for StageCtx<'_> {
    type Target = Uncore;

    fn deref(&self) -> &Uncore {
        self.uncore
    }
}

impl<'a> StageCtx<'a> {
    pub(super) const fn new(
        hart: &'a Hart,
        core: &'a mut CoreUnits,
        uncore: &'a mut Uncore,
    ) -> Self {
        Self { hart, core, uncore }
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
        self.uncore.stats.counter(stat)
    }

    /// The event queue, to schedule a packet.
    #[inline]
    pub const fn events(&mut self) -> &mut EventQueue {
        &mut self.uncore.event_queue
    }

    /// Begins (or completes) translation of a virtual address.
    pub fn translate(
        &mut self,
        vaddr: VirtAddr,
        access: AccessType,
        size: u64,
    ) -> TranslateOutcome {
        memory::translate(self.core, self.hart, self.uncore, vaddr, access, size)
    }

    /// Trains the load prefetcher on a load by `pc` to `vaddr`, which went
    /// to `paddr`, and returns the prefetches to send. A prefetch must lie
    /// in RAM the load could read; the ones that cannot go are counted.
    pub fn train_load_prefetcher(
        &mut self,
        pc: VirtAddr,
        vaddr: VirtAddr,
        paddr: PhysAddr,
    ) -> Vec<LoadPrefetch> {
        let CoreUnits { load_prefetcher, mmu, stat_paths, l1_d_cache, .. } = &mut *self.core;
        let Some(prefetcher) = load_prefetcher.as_mut() else { return Vec::new() };
        let hart = self.hart;
        let privilege = memory::data_privilege(hart);
        let placer = PagePlacer::new(vaddr, paddr, prefetcher.page_boundary(), |target| {
            mmu.prefetch_translation(target, privilege, &hart.csrs)
        });
        let line_bytes = l1_d_cache.line_bytes() as u64;
        let paths = stat_paths.load_prefetch;
        let uncore = &mut *self.uncore;
        let prefetches = prefetcher.train(pc, vaddr, |line| {
            let placed = placer.place(line).and_then(|line_paddr| {
                if !uncore.bus.is_ram(line_paddr, line_bytes) {
                    return Err(PrefetchDrop::NotRam);
                }
                let machine = privilege == PrivilegeMode::Machine;
                let pmp = hart.pmp.check(line_paddr.val(), line_bytes, true, false, false, machine);
                if pmp != PmpResult::Allow {
                    return Err(PrefetchDrop::Denied);
                }
                Ok(line_paddr)
            });
            if let Err(drop) = placed {
                let stat = match drop {
                    PrefetchDrop::PageBoundary => paths.page_boundary,
                    PrefetchDrop::TlbMiss => paths.tlb_miss,
                    PrefetchDrop::Denied => paths.denied,
                    PrefetchDrop::NotRam => paths.not_ram,
                };
                uncore.stats.counter(stat).inc();
            }
            placed
        });
        for prefetch in &prefetches {
            let stat = if prefetch.into == CacheLevel::L1D { paths.l1 } else { paths.l2 };
            uncore.stats.counter(stat).inc();
        }
        prefetches
    }

    /// Resumes a walk that was parked waiting on a PTE response.
    pub fn translate_continue(
        &mut self,
        state: WalkState,
        raw_pte: u64,
        bus_transit_cycles: u64,
    ) -> TranslateOutcome {
        memory::translate_continue(
            self.core,
            self.hart,
            self.uncore,
            state,
            raw_pte,
            bus_transit_cycles,
        )
    }

    /// Sets a leaf PTE's A/D bits as the hardware does, atomically with
    /// the check that the PTE still holds the value its walk found.
    pub fn apply_pte_update(&mut self, update: &PteUpdate) -> PteUpdateOutcome {
        let Some(current) = self.uncore.memory.read(update.pte_addr, 8) else {
            return PteUpdateOutcome::Changed;
        };
        match update.applied_to(current) {
            None => PteUpdateOutcome::Changed,
            Some(pte) if pte == current => PteUpdateOutcome::AlreadySet,
            Some(pte) => {
                let writer = Writer::Hart(self.hart.hart_id);
                self.uncore.publish_write(writer, update.pte_addr, pte, MemWidth::Double);
                PteUpdateOutcome::Written(pte)
            }
        }
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
