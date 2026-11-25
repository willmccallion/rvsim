//! DDR5 JEDEC speed bins.
//!
//! JESD79-5B gives each timing as "the larger of N clocks and T ns";
//! [`Ddr5SpeedBin`] carries those raw pairs and [`Ddr5SpeedBin::timing`]
//! resolves them for the bin's data rate.

use crate::config::dram::timing::{Constraint, DramTiming};

/// Raw JESD79-5B parameters for one DDR5 speed bin and device density.
///
/// Refresh figures (`t_rfc1`, `t_rfc2`, `t_rfcsb`) depend on the device
/// density; the bins here use 16 Gb devices, the most common DDR5 die.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ddr5SpeedBin {
    /// Data rate in mega-transfers per second (command clock is half).
    pub data_rate_mts: u64,
    /// CAS latency in clocks.
    pub cl: u64,
    /// CAS write latency in clocks (CL − 2 in the standard bins).
    pub cwl: u64,
    /// ACT to first column command, same bank.
    pub t_rcd: Constraint,
    /// PRE to ACT, same bank.
    pub t_rp: Constraint,
    /// ACT to PRE, same bank.
    pub t_ras: Constraint,
    /// ACT to ACT, same bank.
    pub t_rc: Constraint,
    /// ACT to ACT, different bank groups.
    pub t_rrd_s: Constraint,
    /// ACT to ACT, same bank group.
    pub t_rrd_l: Constraint,
    /// Column command to column command, different bank groups.
    pub t_ccd_s: Constraint,
    /// Column read to column read, same bank group.
    pub t_ccd_l: Constraint,
    /// Column write to column write, same bank group.
    pub t_ccd_l_wr: Constraint,
    /// Rolling window in which at most four ACTs may issue per rank.
    pub t_faw: Constraint,
    /// End of write burst to READ, different bank groups.
    pub t_wtr_s: Constraint,
    /// End of write burst to READ, same bank group.
    pub t_wtr_l: Constraint,
    /// End of write burst to PRE, same bank.
    pub t_wr: Constraint,
    /// READ to PRE, same bank.
    pub t_rtp: Constraint,
    /// PRE to PRE, same rank.
    pub t_ppd: Constraint,
    /// All-bank refresh duration (`REFab`).
    pub t_rfc1: Constraint,
    /// Fine-granularity (2x) all-bank refresh duration.
    pub t_rfc2: Constraint,
    /// Same-bank refresh duration (`REFsb`).
    pub t_rfcsb: Constraint,
    /// Average interval between all-bank refresh commands.
    pub t_refi: Constraint,
    /// Power-down exit to first valid command.
    pub t_xp: Constraint,
    /// Minimum time in power-down.
    pub t_pd: Constraint,
}

impl Ddr5SpeedBin {
    /// DDR5-4800B (40-39-39), 16 Gb devices. JESD79-5B tables 2xx (speed
    /// bins) and refresh parameters for 16 Gb.
    pub const DDR5_4800B: Self = Self {
        data_rate_mts: 4800,
        cl: 40,
        cwl: 38,
        t_rcd: Constraint::ps(16_250),
        t_rp: Constraint::ps(16_250),
        t_ras: Constraint::ps(32_000),
        t_rc: Constraint::ps(48_250),
        t_rrd_s: Constraint::max(8, 2_500),
        t_rrd_l: Constraint::max(8, 5_000),
        t_ccd_s: Constraint::clocks(8),
        t_ccd_l: Constraint::max(8, 5_000),
        t_ccd_l_wr: Constraint::max(32, 20_000),
        t_faw: Constraint::max(32, 13_333),
        t_wtr_s: Constraint::max(4, 2_500),
        t_wtr_l: Constraint::max(16, 10_000),
        t_wr: Constraint::ps(30_000),
        t_rtp: Constraint::max(12, 7_500),
        t_ppd: Constraint::clocks(2),
        t_rfc1: Constraint::ps(295_000),
        t_rfc2: Constraint::ps(160_000),
        t_rfcsb: Constraint::ps(130_000),
        t_refi: Constraint::ps(3_900_000),
        t_xp: Constraint::max(8, 7_500),
        t_pd: Constraint::max(8, 7_500),
    };

    /// DDR5-5600B (46-45-45), 16 Gb devices.
    pub const DDR5_5600B: Self = Self {
        data_rate_mts: 5600,
        cl: 46,
        cwl: 44,
        t_rcd: Constraint::ps(16_071),
        t_rp: Constraint::ps(16_071),
        t_ras: Constraint::ps(32_000),
        t_rc: Constraint::ps(48_071),
        ..Self::DDR5_4800B
    };

    /// The bin's timing table, in command clocks.
    #[must_use]
    pub const fn timing(&self) -> DramTiming {
        let bin = self;
        let rate = bin.data_rate_mts;
        DramTiming {
            data_rate_mts: rate,
            t_cas: bin.cl,
            t_cwl: bin.cwl,
            t_rcd: bin.t_rcd.cycles(rate),
            t_rp: bin.t_rp.cycles(rate),
            // DDR5 precharges every bank of a rank in tRP too.
            t_rp_all: bin.t_rp.cycles(rate),
            t_ras: bin.t_ras.cycles(rate),
            t_rc: bin.t_rc.cycles(rate),
            t_rrd_s: bin.t_rrd_s.cycles(rate),
            t_rrd_l: bin.t_rrd_l.cycles(rate),
            t_ccd_s: bin.t_ccd_s.cycles(rate),
            t_ccd_l: bin.t_ccd_l.cycles(rate),
            t_ccd_l_wr: bin.t_ccd_l_wr.cycles(rate),
            t_faw: bin.t_faw.cycles(rate),
            t_wtr_s: bin.t_wtr_s.cycles(rate),
            t_wtr_l: bin.t_wtr_l.cycles(rate),
            t_rtw: 2,
            t_wr: bin.t_wr.cycles(rate),
            t_rtp: bin.t_rtp.cycles(rate),
            t_ppd: bin.t_ppd.cycles(rate),
            t_rtrs: 2,
            t_rfc1: bin.t_rfc1.cycles(rate),
            t_rfc2: bin.t_rfc2.cycles(rate),
            t_rfcsb: bin.t_rfcsb.cycles(rate),
            t_refi: bin.t_refi.cycles(rate),
            t_xp: bin.t_xp.cycles(rate),
            t_pd: bin.t_pd.cycles(rate),
            bl_half: 8,
        }
    }
}
