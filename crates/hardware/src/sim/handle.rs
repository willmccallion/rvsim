//! The `Handle` trait every event-receiving component implements, and the
//! `HandleCtx` borrow bundle passed to it on dispatch.
//!
//! A component reacts to incoming packets by mutating its own state and
//! scheduling outgoing events through `ctx.scheduler`. It does not reach
//! into other components — cross-component effects are always packet sends.

use crate::config::Config;
use crate::sim::components::ComponentId;
use crate::sim::events::EventQueue;
use crate::sim::packet::Packet;
use crate::sim::state::global_memory::GlobalMemory;
use crate::sim::stats::Stats;

/// Borrow bundle passed to every `Handle::handle` invocation.
///
/// Carries the bench-side scaffolding a component might need: the event
/// scheduler (to schedule outgoing packets), the stats sink, the simulation
/// config (read-only), the current cycle, and the receiver's own
/// [`ComponentId`] so it can stamp `source` on outgoing packets.
#[derive(Debug)]
pub struct HandleCtx<'a> {
    /// Outgoing event scheduler.
    pub scheduler: &'a mut EventQueue,
    /// Component-rooted stats sink.
    pub stats: &'a mut Stats,
    /// The memory image a request takes effect against when served.
    pub memory: &'a mut GlobalMemory,
    /// Simulator configuration (read-only).
    pub config: &'a Config,
    /// The cycle at which this event is being delivered.
    pub cycle: u64,
    /// `ComponentId` of the component receiving this packet (the `self` of
    /// the `Handle::handle` call). Set by the dispatch loop before invoking
    /// the handler.
    pub self_id: ComponentId,
}

/// Trait every event-driven component implements.
pub trait Handle {
    /// React to `packet` arriving from `source`. May mutate `self` and schedule
    /// further events through `ctx.scheduler`.
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>);
}
