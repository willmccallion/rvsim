//! Per-bank / per-rank / per-subchannel dynamic state for the DDR5 controller.
//!
//! State is mutated by the scheduler as commands issue; timing constraints
//! are enforced by consulting the last-command timestamps stored here.

use std::collections::VecDeque;

use crate::common::{LineAddr, PhysAddr};
use crate::sim::components::{BankGroupId, ComponentId, RankId, ReqId, RowId};
use crate::sim::packet::{AccessSize, MemOp};
use crate::soc::memory::address::DramLocation;
use crate::soc::memory::ddr5::stats::{BankCounters, SubchannelCounters};

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
    /// Cycle at which the bank's in-progress refresh completes; meaningful
    /// while `state` is [`BankState::Refreshing`].
    pub refresh_end: u64,
    /// Command counters, published to the stats tree each controller tick.
    pub counters: BankCounters,
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
            refresh_end: 0,
            counters: BankCounters::new(),
        }
    }
}

impl Default for Bank {
    fn default() -> Self {
        Self::new()
    }
}

/// Bank lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BankState {
    /// No row open, precharged.
    Idle,
    /// A row is open and column commands are legal (subject to tRCD).
    Active,
    /// PRECHARGE issued but tRP has not elapsed.
    Precharging,
    /// Under refresh; no commands may issue to this bank until
    /// [`Bank::refresh_end`].
    Refreshing,
}

/// Rank power state.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PowerState {
    /// Clocked and accepting commands.
    Active,
    /// Entered power-down at `since`; must stay at least tPD and pay tXP
    /// on exit. `with_open_rows` distinguishes active from precharge
    /// power-down.
    PowerDown {
        /// Clock of the power-down entry command.
        since: u64,
        /// True for active power-down (rows left open).
        with_open_rows: bool,
    },
}

/// Where a rank is in its refresh cycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RefreshPhase {
    /// No refresh due.
    Idle,
    /// A refresh is due: the covered banks accept no new commands, open
    /// rows are precharged, then the REFRESH command issues.
    Pending {
        /// Banks the pending refresh covers.
        bank_mask: u64,
        /// Clocks the banks stay busy once the REFRESH issues.
        duration: u64,
    },
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
    /// Cycle at which the next refresh command becomes due.
    pub next_refresh: u64,
    /// Refresh state machine.
    pub refresh_phase: RefreshPhase,
    /// Refresh commands issued so far on this rank; selects the bank set
    /// for same-bank refresh.
    pub refresh_seq: u64,
    /// Cycle at which the last command targeting this rank was issued;
    /// enforces per-rank command-bus serialization without cross-rank
    /// interference. Zero before any command.
    pub last_command_cycle: u64,
    /// Cycle of the most recent PRECHARGE on this rank (tPPD). Zero before
    /// any precharge.
    pub last_precharge: u64,
    /// Power state.
    pub power: PowerState,
    /// Earliest cycle a command may issue after the last power-down exit
    /// (exit + tXP). Zero before any exit.
    pub power_up_at: u64,
}

impl Rank {
    /// Fresh rank with `bank_count` idle banks whose first refresh is due at
    /// `first_refresh`.
    #[must_use]
    pub fn new(bank_count: usize, first_refresh: u64) -> Self {
        Self {
            banks: vec![Bank::new(); bank_count],
            faw: [0; 4],
            faw_slot: 0,
            faw_populated: 0,
            next_refresh: first_refresh,
            refresh_phase: RefreshPhase::Idle,
            refresh_seq: 0,
            last_command_cycle: 0,
            last_precharge: 0,
            power: PowerState::Active,
            power_up_at: 0,
        }
    }

    /// Earliest cycle the rank's command bus accepts a new command: after
    /// the previous command's tenure and after any power-down exit.
    #[must_use]
    pub const fn command_floor(&self) -> u64 {
        if self.power_up_at > self.last_command_cycle {
            self.power_up_at
        } else {
            self.last_command_cycle
        }
    }

    /// True if any bank holds an open row.
    #[must_use]
    pub fn has_open_row(&self) -> bool {
        self.banks.iter().any(|b| b.state == BankState::Active)
    }

    /// True if bank `bank_index` is covered by a pending refresh or is
    /// currently refreshing, so it must not receive new commands.
    #[must_use]
    pub fn bank_held_for_refresh(&self, bank_index: usize) -> bool {
        let pending = match self.refresh_phase {
            RefreshPhase::Idle => false,
            RefreshPhase::Pending { bank_mask, .. } => {
                bank_index < 64 && (bank_mask >> bank_index) & 1 == 1
            }
        };
        pending || self.banks[bank_index].state == BankState::Refreshing
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
    /// A patrol-scrub read generated by the controller itself; it produces
    /// no response.
    pub scrub: bool,
    /// An ACTIVATE was issued on this request's behalf, so its column
    /// command counts as a row miss.
    pub activated: bool,
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
    /// Counters and samples, published to the stats tree each controller tick.
    pub counters: SubchannelCounters,
}

impl Subchannel {
    /// Constructs a fresh subchannel with `rank_count` idle ranks each holding
    /// `bank_count` banks. Per-rank first-refresh cycles are staggered evenly
    /// across `refresh_interval` so multiple ranks do not synchronously
    /// demand the command bus for their first refresh.
    #[must_use]
    pub fn new(rank_count: usize, bank_count: usize, refresh_interval: u64) -> Self {
        Self {
            ranks: fresh_ranks(rank_count, bank_count, refresh_interval, 0),
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
            counters: SubchannelCounters::default(),
        }
    }

    /// Restarts the subchannel's timing as if it powered up at DRAM clock
    /// `origin`: fresh ranks whose refreshes are staggered from there and
    /// no command history. Queued requests and counters are kept.
    pub fn restart_at(&mut self, origin: u64, refresh_interval: u64) {
        let bank_count = self.ranks.first().map_or(0, |rank| rank.banks.len());
        self.ranks = fresh_ranks(self.ranks.len(), bank_count, refresh_interval, origin);
        self.drain_state = WriteDrainState::Filling;
        self.writes_this_drain = 0;
        self.last_command_cycle = 0;
        self.last_data_end = 0;
        self.last_bus_rank = None;
        self.last_data_op = BusOp::None;
        self.last_read_cmd = 0;
        self.last_read_end = 0;
        self.last_write_cmd = 0;
        self.last_write_end = 0;
        self.last_column_bg = None;
    }
}

/// `rank_count` idle ranks powered up at DRAM clock `origin`, their first
/// refreshes staggered evenly across `refresh_interval` so they do not
/// demand the command bus together.
fn fresh_ranks(
    rank_count: usize,
    bank_count: usize,
    refresh_interval: u64,
    origin: u64,
) -> Vec<Rank> {
    let ranks_as_u64 = u64::try_from(rank_count).unwrap_or(u64::MAX);
    let stagger = refresh_interval.checked_div(ranks_as_u64).unwrap_or(0);
    (0..rank_count)
        .map(|r| {
            let offset = u64::try_from(r).unwrap_or(0) * stagger;
            Rank::new(bank_count, origin + refresh_interval + offset)
        })
        .collect()
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
    pub fn new(
        subchannel_count: usize,
        rank_count: usize,
        bank_count: usize,
        refresh_interval: u64,
    ) -> Self {
        Self {
            subchannels: (0..subchannel_count)
                .map(|_| Subchannel::new(rank_count, bank_count, refresh_interval))
                .collect(),
        }
    }
}
