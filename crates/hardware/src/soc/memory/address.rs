//! Physical-address ↔ DRAM-coordinate mapping.
//!
//! [`AddressMapper`] converts a [`PhysAddr`] into a [`DramLocation`] tuple of
//! (channel, subchannel, rank, bank_group, bank, row, column) using one of
//! several bit-interleave strategies. All counts must be powers of two;
//! extraction is a set of table-driven bit-slices with no per-request
//! branching.
//!
//! The low [`CACHE_LINE_OFFSET_BITS`] bits of a physical address are the
//! intra-line byte offset and are not part of any DRAM coordinate — every
//! mapping places its lowest-order field at bit [`CACHE_LINE_OFFSET_BITS`].
//! [`AddressMapper::compose`] therefore reproduces the original address modulo
//! the cache line: `compose(decompose(a)) == a & !((1 << CACHE_LINE_OFFSET_BITS) - 1)`.

use crate::common::PhysAddr;
use crate::config::AddressMappingKind;
use crate::sim::components::{BankGroupId, ChannelId, RankId, RowId, SubchannelId};

/// Number of low-order address bits that are the intra-cache-line byte offset.
/// 64-byte lines → 6 bits.
pub const CACHE_LINE_OFFSET_BITS: u8 = 6;

/// A decomposed physical address in DRAM-topology coordinates.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DramLocation {
    /// Which memory channel.
    pub channel: ChannelId,
    /// Which sub-channel inside the channel.
    pub subchannel: SubchannelId,
    /// Which rank inside the sub-channel.
    pub rank: RankId,
    /// Which bank group inside the rank.
    pub bank_group: BankGroupId,
    /// Bank index inside the bank group.
    pub bank: u8,
    /// Row index inside the bank.
    pub row: RowId,
    /// Column index inside the row (byte granularity).
    pub column: u32,
}

/// Precomputed bit-widths and shifts for a specific mapping configuration.
#[derive(Copy, Clone, Debug)]
pub struct AddressMapper {
    kind: AddressMappingKind,
    channel_bits: u8,
    subchannel_bits: u8,
    rank_bits: u8,
    bank_group_bits: u8,
    bank_bits: u8,
    row_bits: u8,
    column_bits: u8,
    channel_shift: u8,
    subchannel_shift: u8,
    rank_shift: u8,
    bank_group_shift: u8,
    bank_shift: u8,
    row_shift: u8,
    column_shift: u8,
}

impl AddressMapper {
    /// Constructs a mapper for the given topology.
    ///
    /// # Panics
    ///
    /// Panics if any of `channels`, `subchannels`, `ranks`, `bank_groups`, or
    /// `banks_per_group` is not a power of two. Treated as a programmer-error
    /// invariant — callers must validate config before construction.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: AddressMappingKind,
        channels: u8,
        subchannels: u8,
        ranks: u8,
        bank_groups: u8,
        banks_per_group: u8,
        row_bits: u8,
        column_bits: u8,
    ) -> Self {
        assert!(channels.is_power_of_two(), "channels must be a power of two");
        assert!(subchannels.is_power_of_two(), "subchannels must be a power of two");
        assert!(ranks.is_power_of_two(), "ranks must be a power of two");
        assert!(bank_groups.is_power_of_two(), "bank_groups must be a power of two");
        assert!(banks_per_group.is_power_of_two(), "banks_per_group must be a power of two");

        let channel_bits = channels.trailing_zeros() as u8;
        let subchannel_bits = subchannels.trailing_zeros() as u8;
        let rank_bits = ranks.trailing_zeros() as u8;
        let bank_group_bits = bank_groups.trailing_zeros() as u8;
        let bank_bits = banks_per_group.trailing_zeros() as u8;

        let base = CACHE_LINE_OFFSET_BITS;
        let (
            column_shift,
            channel_shift,
            subchannel_shift,
            bank_shift,
            bank_group_shift,
            rank_shift,
            row_shift,
        ) = match kind {
            AddressMappingKind::RoRaBaChCo => {
                // low → high (above cache-line offset):
                //   Column | Subchannel | Channel | Bank | BankGroup | Rank | Row
                let column = base;
                let subchannel = column + column_bits;
                let channel = subchannel + subchannel_bits;
                let bank = channel + channel_bits;
                let bank_group = bank + bank_bits;
                let rank = bank_group + bank_group_bits;
                let row = rank + rank_bits;
                (column, channel, subchannel, bank, bank_group, rank, row)
            }
            AddressMappingKind::RoRaBaCoCh => {
                // low → high (above cache-line offset):
                //   Channel | Subchannel | Column | Bank | BankGroup | Rank | Row
                let channel = base;
                let subchannel = channel + channel_bits;
                let column = subchannel + subchannel_bits;
                let bank = column + column_bits;
                let bank_group = bank + bank_bits;
                let rank = bank_group + bank_group_bits;
                let row = rank + rank_bits;
                (column, channel, subchannel, bank, bank_group, rank, row)
            }
            AddressMappingKind::RoCoRaBaCh => {
                // low → high (above cache-line offset):
                //   Channel | Subchannel | Bank | BankGroup | Rank | Column | Row
                let channel = base;
                let subchannel = channel + channel_bits;
                let bank = subchannel + subchannel_bits;
                let bank_group = bank + bank_bits;
                let rank = bank_group + bank_group_bits;
                let column = rank + rank_bits;
                let row = column + column_bits;
                (column, channel, subchannel, bank, bank_group, rank, row)
            }
        };

        Self {
            kind,
            channel_bits,
            subchannel_bits,
            rank_bits,
            bank_group_bits,
            bank_bits,
            row_bits,
            column_bits,
            channel_shift,
            subchannel_shift,
            rank_shift,
            bank_group_shift,
            bank_shift,
            row_shift,
            column_shift,
        }
    }

    /// The interleave strategy this mapper uses.
    #[inline]
    #[must_use]
    pub const fn kind(&self) -> AddressMappingKind {
        self.kind
    }

    /// Splits a physical address into DRAM coordinates.
    #[must_use]
    pub const fn decompose(&self, addr: PhysAddr) -> DramLocation {
        let raw = addr.val();
        DramLocation {
            channel: ChannelId::new(extract(raw, self.channel_shift, self.channel_bits) as u8),
            subchannel: SubchannelId::new(
                extract(raw, self.subchannel_shift, self.subchannel_bits) as u8,
            ),
            rank: RankId::new(extract(raw, self.rank_shift, self.rank_bits) as u8),
            bank_group: BankGroupId::new(
                extract(raw, self.bank_group_shift, self.bank_group_bits) as u8
            ),
            bank: extract(raw, self.bank_shift, self.bank_bits) as u8,
            row: RowId::new(extract(raw, self.row_shift, self.row_bits) as u32),
            column: extract(raw, self.column_shift, self.column_bits) as u32,
        }
    }

    /// Reassembles a physical address from DRAM coordinates. Inverse of
    /// [`Self::decompose`] modulo field masking.
    #[must_use]
    pub const fn compose(&self, loc: DramLocation) -> PhysAddr {
        let raw = insert(0, self.channel_shift, self.channel_bits, loc.channel.val() as u64)
            | insert(0, self.subchannel_shift, self.subchannel_bits, loc.subchannel.val() as u64)
            | insert(0, self.rank_shift, self.rank_bits, loc.rank.val() as u64)
            | insert(0, self.bank_group_shift, self.bank_group_bits, loc.bank_group.val() as u64)
            | insert(0, self.bank_shift, self.bank_bits, loc.bank as u64)
            | insert(0, self.row_shift, self.row_bits, loc.row.val() as u64)
            | insert(0, self.column_shift, self.column_bits, loc.column as u64);
        PhysAddr::new(raw)
    }
}

#[inline]
const fn mask(bits: u8) -> u64 {
    if bits == 0 { 0 } else { (1u64 << bits) - 1 }
}

#[inline]
const fn extract(raw: u64, shift: u8, bits: u8) -> u64 {
    (raw >> shift) & mask(bits)
}

#[inline]
const fn insert(base: u64, shift: u8, bits: u8, value: u64) -> u64 {
    base | ((value & mask(bits)) << shift)
}
