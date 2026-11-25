//! JEDEC DRAM timing in command clocks, as the command-level controller
//! enforces it whatever the standard.
//!
//! JEDEC standards specify most timings as "the larger of N clocks and T
//! ns": [`Constraint`] carries such a pair and resolves it at a data rate,
//! rounding up as the standards do. Every [`DramTiming`] field is in DRAM
//! command clocks (tCK), measured from the first clock of a command.

/// A JEDEC constraint of the form `max(min_nck × tCK, min_ps)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Constraint {
    /// Minimum number of command clocks.
    pub min_nck: u64,
    /// Minimum absolute time in picoseconds.
    pub min_ps: u64,
}

impl Constraint {
    /// A constraint expressed only in clocks.
    #[must_use]
    pub const fn clocks(min_nck: u64) -> Self {
        Self { min_nck, min_ps: 0 }
    }

    /// A constraint expressed only in picoseconds.
    #[must_use]
    pub const fn ps(min_ps: u64) -> Self {
        Self { min_nck: 0, min_ps }
    }

    /// A constraint with both a clock and an absolute-time floor.
    #[must_use]
    pub const fn max(min_nck: u64, min_ps: u64) -> Self {
        Self { min_nck, min_ps }
    }

    /// Resolves the constraint at `data_rate_mts` (tCK = 2 / data rate),
    /// rounding the absolute-time floor up to whole clocks.
    #[must_use]
    pub const fn cycles(self, data_rate_mts: u64) -> u64 {
        let from_ps = (self.min_ps * data_rate_mts).div_ceil(2_000_000);
        if from_ps > self.min_nck { from_ps } else { self.min_nck }
    }
}

/// Full timing table for one DRAM device, in command clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DramTiming {
    /// Data rate the table was resolved for; fixes tCK for clock-domain
    /// conversion.
    pub data_rate_mts: u64,
    /// CL: cycles from RD command to first data on the bus.
    pub t_cas: u64,
    /// CWL: cycles from WR command to first data on the bus.
    pub t_cwl: u64,
    /// tRCD: cycles from ACT to first RD/WR to the same bank.
    pub t_rcd: u64,
    /// tRP: cycles from PRE to next ACT of the same bank.
    pub t_rp: u64,
    /// tRAS: minimum cycles from ACT to PRE of the same bank.
    pub t_ras: u64,
    /// tRC: cycles from ACT to next ACT of the same bank.
    pub t_rc: u64,
    /// `tRRD_S`: cycles between ACTs to different bank groups.
    pub t_rrd_s: u64,
    /// `tRRD_L`: cycles between ACTs to the same bank group.
    pub t_rrd_l: u64,
    /// `tCCD_S`: cycles between column commands to different bank groups.
    pub t_ccd_s: u64,
    /// `tCCD_L`: cycles between column reads to the same bank group.
    pub t_ccd_l: u64,
    /// `tCCD_L_WR`: cycles between column writes to the same bank group.
    pub t_ccd_l_wr: u64,
    /// tFAW: rolling window in which at most four ACTs may issue per rank.
    pub t_faw: u64,
    /// `tWTR_S`: cycles from end of write data to a READ in a different
    /// bank group.
    pub t_wtr_s: u64,
    /// `tWTR_L`: cycles from end of write data to a READ in the same bank
    /// group.
    pub t_wtr_l: u64,
    /// Read-to-write data-bus turnaround: idle cycles between the end of a
    /// read burst and the start of a write burst. Not a JEDEC parameter;
    /// gem5 uses two clocks.
    pub t_rtw: u64,
    /// tWR: cycles from end of write data to a PRE on the same bank.
    pub t_wr: u64,
    /// tRTP: cycles from RD command to a PRE on the same bank.
    pub t_rtp: u64,
    /// tPPD: cycles between PRE commands on the same rank.
    pub t_ppd: u64,
    /// Rank-to-rank data-bus switch: idle cycles between bursts from
    /// different ranks on one subchannel (gem5's tCS).
    pub t_rtrs: u64,
    /// tRFC1: all-bank refresh duration.
    pub t_rfc1: u64,
    /// tRFC2: fine-granularity all-bank refresh duration.
    pub t_rfc2: u64,
    /// tRFCsb: same-bank refresh duration.
    pub t_rfcsb: u64,
    /// tREFI: average interval between all-bank refresh commands.
    pub t_refi: u64,
    /// tXP: power-down exit to first valid command.
    pub t_xp: u64,
    /// tPD: minimum time in power-down.
    pub t_pd: u64,
    /// BL/2: data-bus cycles occupied by one BL16 burst.
    pub bl_half: u64,
}

impl DramTiming {
    /// Command-clock period in picoseconds, rounded down.
    #[inline]
    #[must_use]
    pub const fn tck_ps(&self) -> u64 {
        2_000_000 / self.data_rate_mts
    }
}

/// A [`DramTiming`] field that configuration may override, named as the
/// JEDEC parameter in snake case (`t_rcd`, `t_ccd_l_wr`, ...).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum DramTimingField {
    TCas,
    TCwl,
    TRcd,
    TRp,
    TRas,
    TRc,
    TRrdS,
    TRrdL,
    TCcdS,
    TCcdL,
    TCcdLWr,
    TFaw,
    TWtrS,
    TWtrL,
    TRtw,
    TWr,
    TRtp,
    TPpd,
    TRtrs,
    TRfc1,
    TRfc2,
    TRfcsb,
    TRefi,
    TXp,
    TPd,
    BlHalf,
}

impl DramTiming {
    /// Overrides one field with a value in command clocks.
    pub const fn set(&mut self, field: DramTimingField, clocks: u64) {
        match field {
            DramTimingField::TCas => self.t_cas = clocks,
            DramTimingField::TCwl => self.t_cwl = clocks,
            DramTimingField::TRcd => self.t_rcd = clocks,
            DramTimingField::TRp => self.t_rp = clocks,
            DramTimingField::TRas => self.t_ras = clocks,
            DramTimingField::TRc => self.t_rc = clocks,
            DramTimingField::TRrdS => self.t_rrd_s = clocks,
            DramTimingField::TRrdL => self.t_rrd_l = clocks,
            DramTimingField::TCcdS => self.t_ccd_s = clocks,
            DramTimingField::TCcdL => self.t_ccd_l = clocks,
            DramTimingField::TCcdLWr => self.t_ccd_l_wr = clocks,
            DramTimingField::TFaw => self.t_faw = clocks,
            DramTimingField::TWtrS => self.t_wtr_s = clocks,
            DramTimingField::TWtrL => self.t_wtr_l = clocks,
            DramTimingField::TRtw => self.t_rtw = clocks,
            DramTimingField::TWr => self.t_wr = clocks,
            DramTimingField::TRtp => self.t_rtp = clocks,
            DramTimingField::TPpd => self.t_ppd = clocks,
            DramTimingField::TRtrs => self.t_rtrs = clocks,
            DramTimingField::TRfc1 => self.t_rfc1 = clocks,
            DramTimingField::TRfc2 => self.t_rfc2 = clocks,
            DramTimingField::TRfcsb => self.t_rfcsb = clocks,
            DramTimingField::TRefi => self.t_refi = clocks,
            DramTimingField::TXp => self.t_xp = clocks,
            DramTimingField::TPd => self.t_pd = clocks,
            DramTimingField::BlHalf => self.bl_half = clocks,
        }
    }
}
