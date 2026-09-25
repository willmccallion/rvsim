//! Per-bank / per-rank / per-subchannel dynamic state for the DDR5 controller.
//!
//! State is mutated by the scheduler as commands issue; timing constraints
//! are enforced by consulting the last-command timestamps stored here.

use std::collections::VecDeque;

use crate::common::{LineAddr, PhysAddr};
use crate::sim::components::{BankGroupId, ComponentId, RankId, ReqId, RowId};
use crate::sim::packet::{AccessSize, MemOp};
use crate::soc::memory::address::DramLocation;

/// One DRAM bank's row-buffer and command-timing state.
#[derive(Clone, Copy, Debug)]
pub struct Bank {
    /// Currently open row, if any.
    pub open_row: Option<RowId>,
    /// Lifecycle marker.
    pub state: BankState,
    /// Cycle of the most recent ACTIVATE on this bank.
    pub last_activate: u64,
    /// Cycle of the most recent PRECHARGE on this bank.
    pub last_precharge: u64,
    /// Cycle of the most recent READ command (not read-end) on this bank.
    pub last_read_cmd: u64,
    /// Cycle at which the last READ's data burst ends on the data bus.
    pub last_read_end: u64,
    /// Cycle of the most recent WRITE command on this bank.
    pub last_write_cmd: u64,
    /// Cycle at which the last WRITE's data burst ends on the data bus.
    pub last_write_end: u64,
}

impl Bank {
    /// Fresh bank in the reset state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            open_row: None,
            state: BankState::Idle,
            last_activate: 0,
            last_precharge: 0,
            last_read_cmd: 0,
            last_read_end: 0,
            last_write_cmd: 0,
            last_write_end: 0,
        }
    }
}

impl Default for Bank {
    fn default() -> Self {
        Self::new()
    }
}

/// Bank lifecycle. `Refreshing` collapses every bank in a rank simultaneously.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BankState {
    /// No row open, precharged.
    Idle,
    /// ACTIVATE issued but tRCD has not elapsed.
    Activating,
    /// A row is open and column commands are legal (subject to tRCD).
    Active,
    /// PRECHARGE issued but tRP has not elapsed.
    Precharging,
    /// Rank-wide refresh; no commands may issue to this bank until tRFC.
    Refreshing,
}

/// Per-rank rolling-window state for tFAW plus refresh scheduling.
#[derive(Clone, Debug)]
pub struct Rank {
    /// Banks belonging to this rank (indexed by `bank_group * banks_per_group + bank`).
    pub banks: Vec<Bank>,
    /// Last four ACTIVATE cycles observed on this rank; slots are overwritten
    /// in round-robin order.
    pub faw: [u64; 4],
    /// Round-robin index into `faw`.
    pub faw_slot: usize,
    /// Number of ACTIVATEs recorded so far, saturating at 4. The tFAW
    /// constraint only kicks in once the window is fully populated.
    pub faw_populated: u8,
    /// Cycle at which the next auto-refresh becomes due.
    pub next_refresh: u64,
    /// Cycle until which the rank is blocked by an in-progress refresh.
    pub refresh_end: u64,
    /// Cycle at which the last command targeting this rank was issued;
    /// enforces per-rank command-bus serialization without cross-rank
    /// interference. Zero before any command.
    pub last_command_cycle: u64,
    /// Cycle of the most recent PRECHARGE on this rank (tPPD). Zero before
    /// any precharge.
    pub last_precharge: u64,
}

impl Rank {
    /// Fresh rank with `bank_count` idle banks.
    #[must_use]
    pub fn new(bank_count: usize, t_refi: u64) -> Self {
        Self {
            banks: vec![Bank::new(); bank_count],
            faw: [0; 4],
            faw_slot: 0,
            faw_populated: 0,
            next_refresh: t_refi,
            refresh_end: 0,
            last_command_cycle: 0,
            last_precharge: 0,
        }
    }

    /// Advances the tFAW rolling window with a new ACTIVATE at `cycle`. Returns
    /// the earliest cycle at which the next ACTIVATE on this rank is legal
    /// (the slot that will be overwritten next).
    pub const fn record_activate(&mut self, cycle: u64, _t_faw: u64) {
        self.faw[self.faw_slot] = cycle;
        self.faw_slot = (self.faw_slot + 1) % 4;
        if self.faw_populated < 4 {
            self.faw_populated += 1;
        }
    }

    /// Earliest cycle at which the next ACTIVATE on this rank may issue given
    /// the current tFAW window. Returns 0 while fewer than four ACTIVATEs
    /// have been observed (the window isn't yet full).
    #[must_use]
    pub const fn earliest_activate_faw(&self, t_faw: u64) -> u64 {
        if self.faw_populated < 4 {
            return 0;
        }
        self.faw[self.faw_slot] + t_faw
    }
}

/// Whether the last data-bus tenure was a read or a write.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BusOp {
    /// Data bus was driven by a READ.
    Read,
    /// Data bus was driven by a WRITE.
    Write,
    /// No tenure yet.
    None,
}

/// Whether the scheduler is accumulating writes or actively draining them.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WriteDrainState {
    /// Below the high-water mark; reads take priority.
    Filling,
    /// Above the high-water mark; writes take priority until the low-water
    /// mark is reached.
    Draining,
}

/// One in-flight request that the scheduler owns.
#[derive(Clone, Debug)]
pub struct PendingReq {
    /// Correlator for the eventual response.
    pub req_id: ReqId,
    /// Arrival at the controller, in DRAM clocks (for latency stats).
    pub arrival_cycle: u64,
    /// Full physical address.
    pub paddr: PhysAddr,
    /// Cache line the request targets; the unit of write merging and
    /// read-hits-write forwarding.
    pub line: LineAddr,
    /// Decomposed DRAM coordinates.
    pub loc: DramLocation,
    /// Access size for the response.
    pub size: AccessSize,
    /// Read / write / fetch.
    pub op: MemOp,
    /// Who to route the response to.
    pub source: ComponentId,
}

/// Per-subchannel scheduler state: split queues, drain state, and command /
/// data bus availability trackers.
#[derive(Clone, Debug)]
pub struct Subchannel {
    /// Ranks visible on this subchannel.
    pub ranks: Vec<Rank>,
    /// Requests that have reached the controller but not yet entered their
    /// queue: either not arrived yet (in DRAM clocks) or blocked by a full
    /// queue. Admitted in arrival order.
    pub inbound: VecDeque<PendingReq>,
    /// FIFO of pending reads.
    pub read_queue: VecDeque<PendingReq>,
    /// FIFO of pending writes.
    pub write_queue: VecDeque<PendingReq>,
    /// Fill-vs-drain policy state.
    pub drain_state: WriteDrainState,
    /// Writes issued since the scheduler last switched to draining.
    pub writes_this_drain: usize,
    /// Cycle at which the command bus is next available.
    pub last_command_cycle: u64,
    /// Cycle at which the data bus is next available.
    pub last_data_end: u64,
    /// Which rank drove the data bus most recently.
    pub last_bus_rank: Option<RankId>,
    /// Kind of the last tenure — enables tWTR/tRTW enforcement at
    /// subchannel granularity.
    pub last_data_op: BusOp,
    /// Cycle of the most recent RD on this subchannel (any rank/bank).
    pub last_read_cmd: u64,
    /// Cycle at which the last RD's data burst ended.
    pub last_read_end: u64,
    /// Cycle of the most recent WR on this subchannel.
    pub last_write_cmd: u64,
    /// Cycle at which the last WR's data burst ended.
    pub last_write_end: u64,
    /// Bank group of the most recent column command (RD or WR); `None` before
    /// the first column command.
    pub last_column_bg: Option<BankGroupId>,
}

impl Subchannel {
    /// Constructs a fresh subchannel with `rank_count` idle ranks each holding
    /// `bank_count` banks. Per-rank first-refresh cycles are staggered evenly
    /// across `t_refi` so multiple ranks do not synchronously demand the
    /// command bus for their first refresh.
    #[must_use]
    pub fn new(rank_count: usize, bank_count: usize, t_refi: u64) -> Self {
        let ranks_as_u64 = u64::try_from(rank_count).unwrap_or(u64::MAX);
        let stagger = if ranks_as_u64 == 0 { 0 } else { t_refi / ranks_as_u64 };
        Self {
            ranks: (0..rank_count)
                .map(|r| {
                    let offset = u64::try_from(r).unwrap_or(0) * stagger;
                    Rank::new(bank_count, t_refi + offset)
                })
                .collect(),
            inbound: VecDeque::new(),
            read_queue: VecDeque::new(),
            write_queue: VecDeque::new(),
            drain_state: WriteDrainState::Filling,
            writes_this_drain: 0,
            last_command_cycle: 0,
            last_data_end: 0,
            last_bus_rank: None,
            last_data_op: BusOp::None,
            last_read_cmd: 0,
            last_read_end: 0,
            last_write_cmd: 0,
            last_write_end: 0,
            last_column_bg: None,
        }
    }
}

/// One DRAM channel: an array of DDR5 subchannels.
#[derive(Clone, Debug)]
pub struct DramChannel {
    /// Subchannels belonging to this channel.
    pub subchannels: Vec<Subchannel>,
}

impl DramChannel {
    /// Constructs a channel with `subchannel_count` fresh subchannels.
    #[must_use]
    pub fn new(subchannel_count: usize, rank_count: usize, bank_count: usize, t_refi: u64) -> Self {
        Self {
            subchannels: (0..subchannel_count)
                .map(|_| Subchannel::new(rank_count, bank_count, t_refi))
                .collect(),
        }
    }
}
