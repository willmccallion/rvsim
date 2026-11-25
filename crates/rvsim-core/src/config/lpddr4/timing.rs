//! LPDDR4 speed bins and their timing in the controller's terms.
//!
//! The values are JESD209-4's as Micron prints them (200b x32 LPDDR4
//! datasheets MT53B384M32D2 rev. E and MT53E1G32D2 rev. F, AC timing
//! tables). LPDDR4 measures a READ's or WRITE's latency from the last clock
//! of its four-clock command (the second edge of CAS-2) and tRCD and tRAS
//! from ACTIVATE-2, two clocks into the ACTIVATE; [`Lpddr4SpeedBin::timing`]
//! moves each to the first clock of the command, which the controller
//! measures from, and turns the datasheet's command-to-command rules into
//! the controller's gaps between bursts.

use crate::config::dram::CommandCycles;
use crate::config::dram::timing::{Constraint, DramTiming};

/// LPDDR4's command bus.
///
/// Every command is two-clock halves: ACTIVATE (ACTIVATE-1, ACTIVATE-2),
/// READ and WRITE (with CAS-2) take four clocks, PRECHARGE and REFRESH two.
/// Power-down is entered and left on CKE, off the command bus.
pub const LPDDR4_COMMAND_CYCLES: CommandCycles =
    CommandCycles { activate: 4, column: 4, precharge: 2, refresh: 2, power_down: 0 };

/// Clocks from the start of an ACTIVATE to ACTIVATE-2.
const ACTIVATE_2_OFFSET: u64 = 2;
/// Clocks from the start of a READ or WRITE to its last clock.
const COLUMN_COMMAND_END: u64 = LPDDR4_COMMAND_CYCLES.column - 1;
/// Clocks a 64-byte line takes on a 16-bit channel: a 32-beat burst.
const BURST_CLOCKS: u64 = 16;

/// Raw datasheet parameters for one LPDDR4 speed bin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lpddr4SpeedBin {
    /// Data rate in mega-transfers per second (command clock is half).
    pub data_rate_mts: u64,
    /// READ latency with DBI off, in clocks.
    pub rl: u64,
    /// WRITE latency, set A, in clocks.
    pub wl: u64,
    /// Latest a read's data strobe follows its clock (tDQSCK max).
    pub t_dqsck_max: Constraint,
    /// Nominal delay of a write's first strobe after its clock (tDQSS).
    pub t_dqss: Constraint,
    /// Write preamble (tWPRE, 1.8 clocks).
    pub t_wpre: Constraint,
    /// ACTIVATE-2 to READ or WRITE.
    pub t_rcd: Constraint,
    /// PRECHARGE of one bank to its next ACTIVATE (tRPpb).
    pub t_rp_pb: Constraint,
    /// PRECHARGE of every bank to the next ACTIVATE (tRPab).
    pub t_rp_ab: Constraint,
    /// ACTIVATE-2 to PRECHARGE.
    pub t_ras: Constraint,
    /// ACTIVATE to ACTIVATE of another bank.
    pub t_rrd: Constraint,
    /// Window in which at most four ACTIVATEs may issue.
    pub t_faw: Constraint,
    /// End of a write's data to a READ.
    pub t_wtr: Constraint,
    /// End of a write's data to a PRECHARGE (write recovery).
    pub t_wr: Constraint,
    /// READ to PRECHARGE.
    pub t_rtp: Constraint,
    /// PRECHARGE to PRECHARGE.
    pub t_ppd: Constraint,
    /// Average interval between all-bank refreshes.
    pub t_refi: Constraint,
    /// Power-down exit to the next command.
    pub t_xp: Constraint,
    /// Minimum time in power-down (tCKE).
    pub t_cke: Constraint,
}

impl Lpddr4SpeedBin {
    /// LPDDR4-3200 (tCK 0.625 ns).
    pub const LPDDR4_3200: Self = Self {
        data_rate_mts: 3200,
        rl: 28,
        wl: 14,
        t_dqsck_max: Constraint::ps(3_500),
        t_dqss: Constraint::clocks(1),
        t_wpre: Constraint::clocks(2),
        t_rcd: Constraint::max(4, 18_000),
        t_rp_pb: Constraint::max(3, 18_000),
        t_rp_ab: Constraint::max(3, 21_000),
        t_ras: Constraint::max(3, 42_000),
        t_rrd: Constraint::max(4, 10_000),
        t_faw: Constraint::ps(40_000),
        t_wtr: Constraint::max(8, 10_000),
        t_wr: Constraint::max(4, 18_000),
        t_rtp: Constraint::max(8, 7_500),
        t_ppd: Constraint::clocks(4),
        t_refi: Constraint::ps(3_904_000),
        t_xp: Constraint::max(5, 7_500),
        t_cke: Constraint::max(4, 7_500),
    };

    /// The bin's timing for dies of `density`, in command clocks from the
    /// first clock of each command.
    #[must_use]
    pub const fn timing(&self, density: Lpddr4Density) -> DramTiming {
        let rate = self.data_rate_mts;
        let dqsck = self.t_dqsck_max.cycles(rate);
        let t_cas = self.rl + COLUMN_COMMAND_END + dqsck;
        let t_cwl = self.wl + COLUMN_COMMAND_END + self.t_dqss.cycles(rate);
        let t_ras = self.t_ras.cycles(rate) + ACTIVATE_2_OFFSET;
        let t_rp = self.t_rp_pb.cycles(rate);
        let write_end = t_cwl + BURST_CLOCKS;
        // WRITE to READ: WL + 1 + BL/2 + RU(tWTR / tCK), from the WRITE.
        let write_to_read = self.wl + 1 + BURST_CLOCKS + self.t_wtr.cycles(rate);
        // WRITE to PRECHARGE: WL + 1 + BL/2 + RU(tWR / tCK), from the WRITE.
        let write_to_precharge = self.wl + 1 + BURST_CLOCKS + self.t_wr.cycles(rate);
        // READ to PRECHARGE: BL/2 + MAX(8, RU(tRTP / tCK)) - 8, from the READ.
        let rtp = self.t_rtp.cycles(rate);
        let rtp_at_least_8 = if rtp > 8 { rtp } else { 8 };
        let read_to_precharge = BURST_CLOCKS + rtp_at_least_8 - 8;
        // READ to WRITE: RL + RU(tDQSCKmax / tCK) + BL/2 - WL + tWPRE
        // + RD(tRPST), from the READ; tRPST (0.4 clocks) rounds down to 0.
        let read_to_write = self.rl + dqsck + BURST_CLOCKS + self.t_wpre.cycles(rate) - self.wl;
        let refresh = density.refresh();
        DramTiming {
            data_rate_mts: rate,
            t_cas,
            t_cwl,
            t_rcd: self.t_rcd.cycles(rate) + ACTIVATE_2_OFFSET,
            t_rp,
            t_rp_all: self.t_rp_ab.cycles(rate),
            t_ras,
            t_rc: t_ras + t_rp,
            t_rrd_s: self.t_rrd.cycles(rate),
            t_rrd_l: self.t_rrd.cycles(rate),
            t_ccd_s: BURST_CLOCKS,
            t_ccd_l: BURST_CLOCKS,
            t_ccd_l_wr: BURST_CLOCKS,
            t_faw: self.t_faw.cycles(rate),
            t_wtr_s: write_to_read.saturating_sub(write_end),
            t_wtr_l: write_to_read.saturating_sub(write_end),
            t_rtw: (read_to_write + t_cwl).saturating_sub(t_cas + BURST_CLOCKS),
            t_wr: write_to_precharge.saturating_sub(write_end),
            t_rtp: read_to_precharge,
            t_ppd: self.t_ppd.cycles(rate),
            // A second rank's strobes differ by tDQSCK_rank2rank (1 ns).
            t_rtrs: Constraint::ps(1_000).cycles(rate),
            t_rfc1: refresh.all_bank.cycles(rate),
            // LPDDR4 has no fine-granularity refresh.
            t_rfc2: refresh.all_bank.cycles(rate),
            t_rfcsb: refresh.per_bank.cycles(rate),
            t_refi: self.t_refi.cycles(rate),
            t_xp: self.t_xp.cycles(rate),
            t_pd: self.t_cke.cycles(rate),
            bl_half: BURST_CLOCKS,
        }
    }
}

/// Density of one die, which serves one 16-bit channel. The 6 Gb and 12 Gb
/// dies have a number of rows that is not a power of two, which the address
/// mapper cannot slice, so they are not offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub enum Lpddr4Density {
    /// 4 Gb: 32,768 rows a bank.
    Gb4,
    /// 8 Gb: 65,536 rows a bank.
    Gb8,
    /// 16 Gb: 131,072 rows a bank.
    #[default]
    Gb16,
}

/// How long a refresh holds the banks it covers.
#[derive(Clone, Copy, Debug)]
struct RefreshTimes {
    all_bank: Constraint,
    per_bank: Constraint,
}

impl Lpddr4Density {
    /// Row-address bits of a bank.
    #[must_use]
    pub const fn row_bits(self) -> u8 {
        match self {
            Self::Gb4 => 15,
            Self::Gb8 => 16,
            Self::Gb16 => 17,
        }
    }

    /// tRFCab and tRFCpb (Micron 200b rev. E, refresh requirements table).
    const fn refresh(self) -> RefreshTimes {
        match self {
            Self::Gb4 => {
                RefreshTimes { all_bank: Constraint::ps(180_000), per_bank: Constraint::ps(90_000) }
            }
            Self::Gb8 => RefreshTimes {
                all_bank: Constraint::ps(280_000),
                per_bank: Constraint::ps(140_000),
            },
            Self::Gb16 => RefreshTimes {
                all_bank: Constraint::ps(380_000),
                per_bank: Constraint::ps(190_000),
            },
        }
    }
}
