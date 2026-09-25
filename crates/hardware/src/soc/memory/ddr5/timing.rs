//! DDR5 JEDEC timing constants.
//!
//! Values are expressed in DRAM command-clock cycles at the DDR5-4800 speed
//! bin (2400 MHz command clock, 4800 MT/s data rate) per JESD79-5B §4.13.
//! The simulator's cycle domain is equated to the command clock at 1:1;
//! operators wanting a different memory speed scale all `t_*` fields
//! proportionally in their configuration.

/// Full JEDEC timing table for a single DDR5 device.
///
/// All fields are in command-clock cycles. The [`Default`] impl returns the
/// DDR5-4800 numbers from JESD79-5B. Individual constraints are documented
/// on the fields themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ddr5Timing {
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
    /// tRC: cycles from ACT to next ACT of the same bank (`t_ras + t_rp`).
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
    ///
    /// DDR5 write-CRC recovery makes this substantially larger than `tCCD_L`
    /// for reads.
    pub t_ccd_l_wr: u64,
    /// tFAW: rolling window in which at most four ACTs may issue.
    pub t_faw: u64,
    /// `tWTR_S`: cycles from end of write data to a following RD in a different
    /// bank group.
    pub t_wtr_s: u64,
    /// `tWTR_L`: cycles from end of write data to a following RD in the same
    /// bank group.
    pub t_wtr_l: u64,
    /// tRTW: cycles from RD command to a following WR command.
    pub t_rtw: u64,
    /// tWR: cycles from end of write data to a PRE on the same bank.
    pub t_wr: u64,
    /// tRTP: cycles from RD command to a PRE on the same bank.
    pub t_rtp: u64,
    /// tRTRS: rank-to-rank data-bus switch cost inside a subchannel.
    pub t_rtrs: u64,
    /// tRFC1: all-bank refresh latency for a 16 Gb device.
    pub t_rfc1: u64,
    /// tRFC2: two-cycle refresh latency (approximate).
    pub t_rfc2: u64,
    /// tREFI: average interval between all-bank refresh commands.
    pub t_refi: u64,
    /// BL/2: data-bus cycles occupied by one BL16 burst.
    pub bl_half: u64,
}

impl Ddr5Timing {
    /// DDR5-4800 defaults from JESD79-5B §4.13.
    #[must_use]
    pub const fn ddr5_4800() -> Self {
        Self {
            t_cas: 40,
            t_cwl: 38,
            t_rcd: 40,
            t_rp: 40,
            t_ras: 77,
            t_rc: 117,
            t_rrd_s: 8,
            t_rrd_l: 8,
            t_ccd_s: 8,
            t_ccd_l: 8,
            t_ccd_l_wr: 32,
            t_faw: 32,
            t_wtr_s: 4,
            t_wtr_l: 12,
            t_rtw: 8,
            t_wr: 30,
            t_rtp: 12,
            t_rtrs: 2,
            t_rfc1: 984,
            t_rfc2: 528,
            t_refi: 3900,
            bl_half: 8,
        }
    }

    /// Effective refresh latency for the all-bank refresh mode. Alias for
    /// [`Self::t_rfc1`].
    #[inline]
    #[must_use]
    pub const fn t_rfc(&self) -> u64 {
        self.t_rfc1
    }
}

impl Default for Ddr5Timing {
    fn default() -> Self {
        Self::ddr5_4800()
    }
}
