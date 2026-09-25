//! Configuration for [`crate::soc::memory::ddr5::controller::Ddr5Controller`].

use crate::soc::memory::address::AddressMappingKind;
use crate::soc::memory::ddr5::timing::Ddr5Timing;

/// Static topology and policy parameters for a DDR5 memory subsystem.
///
/// Every count except the row/column bit widths must be a power of two so that
/// the address mapper can extract fields with pure bit-slice arithmetic. The
/// simulator's cycle domain is treated as the DRAM command clock at 1:1;
/// scaling to a different speed bin is done by scaling every field of
/// [`Ddr5Timing`] together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ddr5Config {
    /// Number of independent DRAM channels.
    pub channels: u8,
    /// Sub-channels per channel (DDR5 defines two 32-bit sub-channels).
    pub subchannels_per_channel: u8,
    /// Ranks per channel (each sub-channel sees the same rank population).
    pub ranks_per_channel: u8,
    /// Bank groups per rank.
    pub bank_groups_per_rank: u8,
    /// Banks per bank group.
    pub banks_per_group: u8,
    /// Number of row-address bits.
    pub row_bits: u8,
    /// Number of column-address bits.
    pub column_bits: u8,
    /// Row size in bytes (implied by column bits + intra-row column stride,
    /// carried explicitly for stats and validation).
    pub row_size_bytes: u32,
    /// Depth of the write queue at which the scheduler starts draining writes.
    pub write_high_watermark: usize,
    /// Depth of the write queue at which the scheduler returns to reads.
    pub write_low_watermark: usize,
    /// Address-bit interleave strategy.
    pub address_mapping: AddressMappingKind,
    /// Command-timing constants.
    pub timing: Ddr5Timing,
}

impl Ddr5Config {
    /// Sane defaults for a small DDR5-4800 desktop configuration: two
    /// channels, two sub-channels each, two ranks per channel, eight bank
    /// groups × four banks, 15-bit rows, 10-bit columns.
    #[must_use]
    pub const fn ddr5_4800() -> Self {
        Self {
            channels: 2,
            subchannels_per_channel: 2,
            ranks_per_channel: 2,
            bank_groups_per_rank: 8,
            banks_per_group: 4,
            row_bits: 15,
            column_bits: 10,
            row_size_bytes: 8192,
            write_high_watermark: 32,
            write_low_watermark: 8,
            address_mapping: AddressMappingKind::RoRaBaChCo,
            timing: Ddr5Timing::ddr5_4800(),
        }
    }
}

impl Default for Ddr5Config {
    fn default() -> Self {
        Self::ddr5_4800()
    }
}
