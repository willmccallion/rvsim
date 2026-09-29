//! What a pipeline works on.
//!
//! [`CoreCtx`] is commit's view: one hart, its core's private
//! micro-architecture and the shared uncore, all mutable. [`StageCtx`] is
//! every other stage's view, with the hart read-only. The system builds a
//! `CoreCtx` from disjoint borrows, so a core cannot reach another core's state,
//! and `CoreCtx::stage` narrows it to a `StageCtx`.

pub mod csr;

pub mod execution;

pub mod memory;

pub mod stage;

pub mod trap;

pub use stage::StageCtx;

use std::ops::{Deref, DerefMut};

use crate::arch::Hart;
use crate::common::PhysAddr;
use crate::isa::op::MemWidth;
use crate::sim::memory::write_log::Writer;
use crate::sim::stats::paths::HartPaths;
use crate::soc::uncore::Uncore;
use crate::uarch::CoreUnits;

/// What a pipeline works on: its hart, its core, and the uncore.
///
/// Built by [`crate::system::SystemState::core_ctx`] from disjoint borrows. Derefs to
/// [`Uncore`] so uncore fields read as `ctx.bus`, `ctx.event_queue`.
#[derive(Debug)]
pub struct CoreCtx<'a> {
    /// The hart the pipeline is executing.
    pub hart: &'a mut Hart,
    /// The pipeline's private micro-architecture.
    pub core: &'a mut CoreUnits,
    /// The uncore.
    pub uncore: &'a mut Uncore,
}

impl Deref for CoreCtx<'_> {
    type Target = Uncore;

    fn deref(&self) -> &Uncore {
        self.uncore
    }
}

impl DerefMut for CoreCtx<'_> {
    fn deref_mut(&mut self) -> &mut Uncore {
        self.uncore
    }
}

impl CoreCtx<'_> {
    /// The view a stage other than commit works on: the hart read-only,
    /// the core and the uncore's stats and event queue mutable.
    #[inline]
    pub const fn stage(&mut self) -> StageCtx<'_> {
        StageCtx::new(self.hart, self.core, self.uncore)
    }

    /// Stat paths of the hart this view executes.
    #[inline]
    #[must_use]
    pub fn hart_paths(&self) -> HartPaths {
        self.uncore.hart_stat_paths[self.hart.hart_id.as_index()]
    }

    /// Sets a load reservation for this hart at `addr` (cache-line aligned).
    #[inline]
    pub fn set_reservation(&mut self, addr: PhysAddr) {
        let hart = self.hart.hart_id;
        self.uncore.memory.reservations_mut().set(hart, addr);
    }

    /// Returns `true` when this hart holds a reservation covering `addr`.
    #[inline]
    pub fn check_reservation(&self, addr: PhysAddr) -> bool {
        self.uncore.memory.reservations().check(self.hart.hart_id, addr)
    }

    /// Clears this hart's load reservation.
    #[inline]
    pub fn clear_reservation(&mut self) {
        let hart = self.hart.hart_id;
        self.uncore.memory.reservations_mut().clear(hart);
    }

    /// Makes `data` visible at `paddr` as a write by this hart. See
    /// [`Uncore::publish_write`].
    #[inline]
    pub fn publish_write(&mut self, paddr: PhysAddr, data: u64, width: MemWidth) {
        let writer = Writer::Hart(self.hart.hart_id);
        self.uncore.publish_write(writer, paddr, data, width);
    }
}
