//! Simulator control: lets software in the guest mark the region it wants
//! measured, as gem5's m5 operations do.
//!
//! # Registers
//!
//! * `0x00`: `COMMAND` (write): `1` resets the stats, `2` dumps them
//!   labelled with `ARG`, `3` ends the simulation with `ARG` as the exit
//!   code, `4` stops the host's run at this point with `ARG` as the label.
//! * `0x08`: `ARG` (read/write): the argument of the next command.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::common::LineAddr;
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData};
use crate::soc::devices::Device;

const COMMAND: u64 = 0x00;
const ARG: u64 = 0x08;

const RESET_STATS: u64 = 1;
const DUMP_STATS: u64 = 2;
const EXIT: u64 = 3;
const BREAK: u64 = 4;

/// A request from the guest that the simulator carries out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimOp {
    /// Zero every stat and start counting from here.
    ResetStats,
    /// Keep a copy of the stats, labelled.
    DumpStats {
        /// The guest's label for this dump.
        label: u64,
    },
    /// Stop the host's run here, so it can inspect, save or switch.
    Break {
        /// The guest's label for this point.
        label: u64,
    },
}

/// The simulator control device.
#[derive(Debug)]
pub struct SimControl {
    base_addr: u64,
    arg: u64,
    pending: Vec<SimOp>,
    exit_signal: Arc<AtomicU64>,
}

impl SimControl {
    /// A device at `base_addr` that ends the simulation through
    /// `exit_signal`.
    pub const fn new(base_addr: u64, exit_signal: Arc<AtomicU64>) -> Self {
        Self { base_addr, arg: 0, pending: Vec::new(), exit_signal }
    }

    fn command(&mut self, command: u64) {
        match command {
            RESET_STATS => self.pending.push(SimOp::ResetStats),
            DUMP_STATS => self.pending.push(SimOp::DumpStats { label: self.arg }),
            EXIT => self.exit_signal.store(self.arg, Ordering::Relaxed),
            BREAK => self.pending.push(SimOp::Break { label: self.arg }),
            _ => {}
        }
    }
}

impl Handle for SimControl {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        let Packet::MemReq { req_id, paddr, size, op, .. } = packet else { return };
        let offset = paddr.val().saturating_sub(self.base_addr);
        let whole_register = matches!(size, AccessSize::B4 | AccessSize::B8);
        let mut value = 0;
        match op {
            MemOp::Write { data: WriteData::Small(written), .. } if whole_register => {
                match offset {
                    COMMAND => self.command(written),
                    ARG => self.arg = written,
                    _ => {}
                }
            }
            MemOp::Read if offset == ARG => value = self.arg,
            _ => {}
        }
        ctx.scheduler.schedule(
            ctx.cycle + ctx.config.system.device_access_cycles(self.name()),
            source,
            ctx.self_id,
            Packet::MemResp {
                req_id,
                line_addr: LineAddr::from_phys(paddr, 64),
                data: MemRespData::Small(value),
                hit_level: HitLevel::Mmio,
                state: MesiState::Exclusive,
            },
        );
    }
}

impl Device for SimControl {
    fn name(&self) -> &'static str {
        "SimControl"
    }

    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x1000)
    }

    fn quiet_ticks(&self) -> Option<u64> {
        None
    }

    fn take_sim_ops(&mut self) -> Vec<SimOp> {
        std::mem::take(&mut self.pending)
    }
}
